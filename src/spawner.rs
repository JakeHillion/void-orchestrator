use log::{debug, error, info};

use super::specification::{Arg, Entrypoint, FileSocket, Pipe, Specification, Trigger};
use super::{PipePair, SocketPair};
use crate::void::VoidBuilder;
use crate::{Error, Result};

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::path::PathBuf;

use nix::sys::socket::{recvmsg, ControlMessageOwned, MsgFlags};
use nix::unistd;

const BUFFER_SIZE: usize = 1024;

pub struct Spawner<'a> {
    pub spec: &'a Specification,
    pub binary: &'a str,
    pub trailing: &'a Vec<&'a str>,

    pub pipes: HashMap<String, PipePair>,
    pub sockets: HashMap<String, SocketPair>,
}

enum TriggerData<'a> {
    /// No data, for example a Startup trigger
    None,

    /// A string sent across a pipe
    Pipe(&'a str),

    /// File(s) sent over a file socket
    FileSocket(Vec<File>),
}

impl<'a> TriggerData<'a> {
    fn args(&mut self) -> Vec<CString> {
        match self {
            TriggerData::None => vec![],
            TriggerData::Pipe(s) => vec![CString::new(s.to_string()).unwrap()],
            TriggerData::FileSocket(fs) => fs
                .drain(..)
                .map(|f| CString::new(f.into_raw_fd().to_string()).unwrap())
                .collect(),
        }
    }
}

impl<'a> Spawner<'a> {
    pub fn spawn(&mut self) -> Result<()> {
        for (name, entrypoint) in &self.spec.entrypoints {
            info!("spawning entrypoint `{}`", name.as_str());

            match &entrypoint.trigger {
                Trigger::Startup => {
                    let binary = PathBuf::from(self.binary).canonicalize()?;

                    let mut builder = VoidBuilder::new();
                    builder.mount(binary, "/entrypoint");

                    let closure = || {
                        let args = self
                            .prepare_args(name, &entrypoint.args, &mut TriggerData::None)
                            .unwrap();

                        if let Err(e) = unistd::execv(&CString::new("/entrypoint").unwrap(), &args)
                            .map_err(|e| Error::Nix {
                                msg: "execv",
                                src: e,
                            })
                        {
                            error!("error: {}", e);
                            1
                        } else {
                            0
                        }
                    };

                    builder.spawn(closure)?;
                }

                Trigger::Pipe(s) => {
                    let pipe = self.pipes.get_mut(s).unwrap().take_read()?;
                    let binary = PathBuf::from(self.binary).canonicalize()?;

                    let mut builder = VoidBuilder::new();
                    builder.mount(binary, "/entrypoint");
                    builder.keep_fd(&pipe);

                    let closure = || match self.pipe_trigger(pipe, entrypoint, name) {
                        Ok(()) => std::process::exit(exitcode::OK),
                        Err(e) => {
                            error!("error in pipe_trigger: {}", e);
                            std::process::exit(1)
                        }
                    };

                    builder.spawn(closure)?;
                }

                Trigger::FileSocket(s) => {
                    let socket = self.sockets.get_mut(s).unwrap().take_read()?;
                    let binary = PathBuf::from(self.binary).canonicalize()?;

                    let mut builder = VoidBuilder::new();
                    builder.mount(binary, "/entrypoint");
                    builder.keep_fd(&socket);

                    let closure = || match self.file_socket_trigger(socket, entrypoint, name) {
                        Ok(()) => std::process::exit(exitcode::OK),
                        Err(e) => {
                            error!("error in file_socket_trigger: {}", e);
                            std::process::exit(1)
                        }
                    };

                    builder.spawn(closure)?;
                }
            }
        }

        Ok(())
    }

    fn pipe_trigger(&self, mut pipe: File, spec: &Entrypoint, name: &str) -> Result<()> {
        let mut buf = [0_u8; BUFFER_SIZE];
        loop {
            let read_bytes = pipe.read(&mut buf)?;
            if read_bytes == 0 {
                return Ok(());
            }

            debug!("triggering from pipe read");

            let mut builder = VoidBuilder::new();
            builder.mount("/entrypoint", "/entrypoint");

            let closure =
                || {
                    let pipe_trigger = std::str::from_utf8(&buf[0..read_bytes]).unwrap();
                    let args = self
                        .prepare_args_ref(name, &spec.args, &mut TriggerData::Pipe(pipe_trigger))
                        .unwrap();

                    if let Err(e) = unistd::execv(&CString::new("/entrypoint").unwrap(), &args)
                        .map_err(|e| Error::Nix {
                            msg: "execv",
                            src: e,
                        })
                    {
                        error!("error: {}", e);
                        1
                    } else {
                        0
                    }
                };

            builder.spawn(closure)?;
        }
    }

    fn file_socket_trigger(&self, socket: File, spec: &Entrypoint, name: &str) -> Result<()> {
        let mut buf = Vec::new();
        loop {
            let msg = recvmsg(socket.as_raw_fd(), &[], Some(&mut buf), MsgFlags::empty()).map_err(
                |e| Error::Nix {
                    msg: "recvmsg",
                    src: e,
                },
            )?;

            debug!("triggering from socket recvmsg");

            for cmsg in msg.cmsgs() {
                match cmsg {
                    ControlMessageOwned::ScmRights(fds) => {
                        let fds = fds
                            .into_iter()
                            .map(|fd| unsafe { File::from_raw_fd(fd) })
                            .collect();

                        let mut builder = VoidBuilder::new();
                        builder.mount("/entrypoint", "/entrypoint");
                        for fd in &fds {
                            builder.keep_fd(fd);
                        }

                        let closure = || {
                            let args = self
                                .prepare_args_ref(
                                    name,
                                    &spec.args,
                                    &mut TriggerData::FileSocket(fds),
                                )
                                .unwrap();

                            if let Err(e) =
                                unistd::execv(&CString::new("/entrypoint").unwrap(), &args).map_err(
                                    |e| Error::Nix {
                                        msg: "execv",
                                        src: e,
                                    },
                                )
                            {
                                error!("error: {}", e);
                                1
                            } else {
                                0
                            }
                        };

                        builder.spawn(closure)?;
                    }
                    _ => unimplemented!(),
                }
            }
        }
    }

    fn prepare_args(
        &mut self,
        entrypoint: &str,
        args: &[Arg],
        trigger: &mut TriggerData,
    ) -> Result<Vec<CString>> {
        let mut out = Vec::new();
        for arg in args {
            out.extend(self.prepare_arg(entrypoint, arg, trigger)?);
        }

        Ok(out)
    }

    fn prepare_args_ref(
        &self,
        entrypoint: &str,
        args: &[Arg],
        trigger: &mut TriggerData,
    ) -> Result<Vec<CString>> {
        let mut out = Vec::new();
        for arg in args {
            out.extend(self.prepare_arg_ref(entrypoint, arg, trigger)?);
        }

        Ok(out)
    }

    fn prepare_arg(
        &mut self,
        entrypoint: &str,
        arg: &Arg,
        trigger: &mut TriggerData,
    ) -> Result<Vec<CString>> {
        match arg {
            Arg::Pipe(p) => match p {
                Pipe::Rx(s) => {
                    let pipe = self.pipes.get_mut(s).unwrap().take_read()?;
                    Ok(vec![CString::new(pipe.into_raw_fd().to_string()).unwrap()])
                }
                Pipe::Tx(s) => {
                    let pipe = self.pipes.get_mut(s).unwrap().take_write()?;
                    Ok(vec![CString::new(pipe.into_raw_fd().to_string()).unwrap()])
                }
            },

            Arg::FileSocket(s) => match s {
                FileSocket::Rx(s) => {
                    let pipe = self.sockets.get_mut(s).unwrap().take_read()?;
                    Ok(vec![CString::new(pipe.into_raw_fd().to_string()).unwrap()])
                }
                FileSocket::Tx(s) => {
                    let pipe = self.sockets.get_mut(s).unwrap().take_write()?;
                    Ok(vec![CString::new(pipe.into_raw_fd().to_string()).unwrap()])
                }
            },

            a => self.prepare_arg_ref(entrypoint, a, trigger),
        }
    }

    fn prepare_arg_ref(
        &self,
        entrypoint: &str,
        arg: &Arg,
        trigger: &mut TriggerData,
    ) -> Result<Vec<CString>> {
        match arg {
            Arg::BinaryName => Ok(vec![CString::new(self.binary).unwrap()]),
            Arg::Entrypoint => Ok(vec![CString::new(entrypoint).unwrap()]),

            Arg::Pipe(p) => Err(Error::BadPipe(p.get_name().to_string())),
            Arg::FileSocket(s) => Err(Error::BadFileSocket(s.get_name().to_string())),

            Arg::File(p) => {
                let f = File::open(p)?.into_raw_fd();
                Ok(vec![CString::new(f.to_string()).unwrap()])
            }

            Arg::Trigger => Ok(trigger.args()),

            Arg::TcpListener { addr } => {
                let listener = TcpListener::bind(addr)?;
                let listener = listener.into_raw_fd();

                Ok(vec![CString::new(listener.to_string()).unwrap()])
            }

            Arg::Trailing => Ok(self
                .trailing
                .iter()
                .map(|s| CString::new(*s).unwrap())
                .collect()),
        }
    }
}
