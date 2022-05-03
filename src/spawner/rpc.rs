use log::{debug, error};

use crate::specification::{AddressFamily as SpecAddressFamily, RpcSpecification};
use crate::Error;

use std::ffi::CStr;
use std::fs::File;
use std::net::{TcpStream, UdpSocket};
use std::os::raw::c_char;
use std::os::unix::io::AsRawFd;

use nix::sys::socket::AddressFamily;
use nix::sys::socket::{recv, send, sendmsg, ControlMessage, MsgFlags};

const MAX_MSG_LENGTH: usize = 4096;

pub struct RpcHandler<'a> {
    permitted_rpcs: &'a [RpcSpecification],
}

impl<'a> RpcHandler<'a> {
    pub(super) fn new(permitted_rpcs: &'a [RpcSpecification]) -> Self {
        Self { permitted_rpcs }
    }

    pub(super) fn handle(&self, socket: File) -> Result<(), Error> {
        let mut buf = vec![0; MAX_MSG_LENGTH];

        loop {
            let read_bytes =
                recv(socket.as_raw_fd(), &mut buf, MsgFlags::empty()).map_err(|e| Error::Nix {
                    msg: "recvmsg",
                    src: e,
                })?;

            debug!("handling rpc");

            if read_bytes < 4 {
                error!("received rpc too short");
                continue;
            }

            // SAFETY: safe as the enum repr is non_exhaustive so any value is valid and the buffer is long enough
            let kind = unsafe { *(buf.as_ptr() as *const RpcKind) };

            let fds = Vec::new();
            if kind.num_fds() > 0 {
                // get any fds to go alongside the message
                // nothing which requires this currently exists
                unimplemented!()
            }

            let resp = handle_rpc(self.permitted_rpcs, kind, &buf[4..], &fds);

            let (msg, fds) = RpcResultSend::new(resp);

            // sendmsg first so its there when listening for the send
            if !fds.is_empty() {
                let fds: Box<[i32]> = fds.iter().map(|f| f.as_raw_fd()).collect();

                sendmsg::<()>(
                    socket.as_raw_fd(),
                    &[],
                    &[ControlMessage::ScmRights(&fds)],
                    MsgFlags::empty(),
                    None,
                )
                .map_err(|e| Error::Nix {
                    msg: "sendmsg",
                    src: e,
                })?;
            }

            // SAFETY: safe as msg is of fixed size
            let msg = unsafe {
                std::slice::from_raw_parts(
                    &msg as *const RpcResultSend as *const u8,
                    std::mem::size_of_val(&msg),
                )
            };

            send(socket.as_raw_fd(), msg, MsgFlags::empty()).map_err(|e| Error::Nix {
                msg: "send",
                src: e,
            })?;
        }
    }
}

#[repr(u32)]
#[non_exhaustive]
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum RpcKind {
    OpenTcpSocket,
    OpenUdpSocket,
}

impl RpcKind {
    fn num_fds(&self) -> usize {
        match self {
            RpcKind::OpenTcpSocket => 0,
            RpcKind::OpenUdpSocket => 0,
        }
    }
}

pub struct OpenSocket {
    pub family: AddressFamily,
    pub port: u16,
    pub host: [c_char],
}

pub enum RpcResult {
    OpenTcpSocket { socket: TcpStream },
    OpenUdpSocket { socket: UdpSocket },

    Error { error: RpcError },
}

pub enum RpcResultSend {
    OpenTcpSocket,
    OpenUdpSocket,

    Error { error: RpcError },
}

impl RpcResultSend {
    fn new(from: RpcResult) -> (Self, Vec<Box<dyn AsRawFd>>) {
        match from {
            RpcResult::OpenTcpSocket { socket } => (Self::OpenTcpSocket, vec![Box::new(socket)]),
            RpcResult::OpenUdpSocket { socket } => (Self::OpenUdpSocket, vec![Box::new(socket)]),
            RpcResult::Error { error } => (Self::Error { error }, vec![]),
        }
    }
}

#[repr(C)]
pub enum RpcError {
    BadlyFormedRequest,
    OperationNotPermitted,
    Io { errno: i32 },
}

fn handle_rpc(
    permitted_rpcs: &[RpcSpecification],
    kind: RpcKind,
    data: &[u8],
    _fds: &[File],
) -> RpcResult {
    fn inner(
        permitted_rpcs: &[RpcSpecification],
        kind: RpcKind,
        data: &[u8],
    ) -> Result<RpcResult, RpcError> {
        match kind {
            RpcKind::OpenTcpSocket => {
                let data = unsafe { &*(data as *const [u8] as *const OpenSocket) };
                if !validate_open_tcp_socket(permitted_rpcs, data)? {
                    Ok(RpcResult::Error {
                        error: RpcError::OperationNotPermitted,
                    })
                } else {
                    handle_open_tcp_socket(data)
                }
            }
            RpcKind::OpenUdpSocket => {
                let data = unsafe { &*(data as *const [u8] as *const OpenSocket) };
                if !validate_open_udp_socket(permitted_rpcs, data)? {
                    Ok(RpcResult::Error {
                        error: RpcError::OperationNotPermitted,
                    })
                } else {
                    handle_open_udp_socket(data)
                }
            }
        }
    }

    match inner(permitted_rpcs, kind, data) {
        Ok(o) => o,
        Err(e) => RpcResult::Error { error: e },
    }
}

fn validate_open_tcp_socket(
    permitted_rpcs: &[RpcSpecification],
    req: &OpenSocket,
) -> Result<bool, RpcError> {
    for each in permitted_rpcs {
        if let RpcSpecification::OpenTcpSocket { family, port, host } = each {
            let mut allowed = true;

            allowed &= match family {
                None => true,
                Some(fam) => match req.family {
                    AddressFamily::Inet => *fam == SpecAddressFamily::Inet,
                    AddressFamily::Inet6 => *fam == SpecAddressFamily::Inet6,
                    _ => false,
                },
            };

            allowed &= match port {
                None => true,
                Some(p) => req.port == *p,
            };

            allowed &= match host {
                None => true,
                Some(h) => {
                    CStr::from_bytes_with_nul(as_u8_slice(&req.host))
                        .map_err(|_| RpcError::BadlyFormedRequest)?
                        .to_string_lossy()
                        .as_ref()
                        == h
                }
            };

            if allowed {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn handle_open_tcp_socket(req: &OpenSocket) -> Result<RpcResult, RpcError> {
    let host = CStr::from_bytes_with_nul(as_u8_slice(&req.host))
        .map_err(|_| RpcError::BadlyFormedRequest)?;
    let host = host.to_str().map_err(|_| RpcError::BadlyFormedRequest)?;

    let socket = TcpStream::connect(host).map_err(|e| RpcError::Io {
        errno: e.raw_os_error().unwrap(),
    })?;

    Ok(RpcResult::OpenTcpSocket { socket })
}

fn validate_open_udp_socket(
    permitted_rpcs: &[RpcSpecification],
    req: &OpenSocket,
) -> Result<bool, RpcError> {
    for each in permitted_rpcs {
        if let RpcSpecification::OpenUdpSocket { family, port, host } = each {
            let mut allowed = true;

            allowed &= match family {
                None => true,
                Some(fam) => match req.family {
                    AddressFamily::Inet => *fam == SpecAddressFamily::Inet,
                    AddressFamily::Inet6 => *fam == SpecAddressFamily::Inet6,
                    _ => false,
                },
            };

            allowed &= match port {
                None => true,
                Some(p) => req.port == *p,
            };

            allowed &= match host {
                None => true,
                Some(h) => {
                    CStr::from_bytes_with_nul(as_u8_slice(&req.host))
                        .map_err(|_| RpcError::BadlyFormedRequest)?
                        .to_string_lossy()
                        .as_ref()
                        == h
                }
            };

            if allowed {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn handle_open_udp_socket(req: &OpenSocket) -> Result<RpcResult, RpcError> {
    let host = CStr::from_bytes_with_nul(as_u8_slice(&req.host))
        .map_err(|_| RpcError::BadlyFormedRequest)?;
    let host = host.to_str().map_err(|_| RpcError::BadlyFormedRequest)?;

    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| RpcError::Io {
        errno: e.raw_os_error().unwrap(),
    })?;

    socket.connect(host).map_err(|e| RpcError::Io {
        errno: e.raw_os_error().unwrap(),
    })?;

    Ok(RpcResult::OpenUdpSocket { socket })
}

fn as_u8_slice(s: &[c_char]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, s.len()) }
}
