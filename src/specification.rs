use log::debug;

use crate::{Error, Result};

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;

use ipnetwork::{Ipv4Network, Ipv6Network};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Specification {
    pub entrypoints: HashMap<String, Entrypoint>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Entrypoint {
    #[serde(default)]
    pub trigger: Trigger,

    #[serde(default = "Arg::default_vec")]
    pub args: Vec<Arg>,

    #[serde(default)]
    pub environment: HashSet<Environment>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Trigger {
    /// Start this entrypoint at application startup
    Startup,

    /// Trigger this entrypoint when a named pipe receives data
    Pipe(String),

    /// Trigger this entrypoint when a named file socket receives data
    FileSocket(String),
}

impl Default for Trigger {
    fn default() -> Self {
        Self::Startup
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub enum Arg {
    /// The binary name, or argv[0], of the original program start
    BinaryName,

    /// The name of this entrypoint
    Entrypoint,

    /// A file descriptor for a file on the filesystem in the launching namespace
    File(PathBuf),

    /// A chosen end of a named pipe
    Pipe(Pipe),

    /// File socket
    FileSocket(FileSocket),

    /// A value specified by the trigger
    /// NOTE: Only valid if the trigger is of type Pipe(...) or FileSocket(...)
    Trigger,

    /// A TCP Listener
    TcpListener { addr: SocketAddr },

    /// An RPC socket that accepts specified commands
    Rpc(Vec<RpcSpecification>),

    /// The rest of argv[1..], 0 or more arguments
    Trailing,
}

impl Arg {
    fn default_vec() -> Vec<Arg> {
        vec![Arg::BinaryName]
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub enum RpcSpecification {
    /// Open a TCP socket
    ///
    /// None for each value means that any value is allowed in the call.
    /// A specified value restricts to exactly that.
    OpenTcpSocket {
        family: Option<AddressFamily>,
        port: Option<u16>,
        host: Option<String>,
    },

    /// Open a UDP socket
    ///
    /// None for each value means that any value is allowed in the call.
    /// A specified value restricts to exactly that.
    OpenUdpSocket {
        family: Option<AddressFamily>,
        port: Option<u16>,
        host: Option<String>,
    },
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub enum AddressFamily {
    /// IPv4 address
    Inet,

    /// IPv6 address
    Inet6,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum Pipe {
    Rx(String),
    Tx(String),
}

impl Pipe {
    pub fn get_name(&self) -> &str {
        match self {
            Pipe::Rx(n) => n,
            Pipe::Tx(n) => n,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum FileSocket {
    Rx(String),
    Tx(String),
}

impl FileSocket {
    pub fn get_name(&self) -> &str {
        match self {
            FileSocket::Rx(n) => n,
            FileSocket::Tx(n) => n,
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Debug)]
pub enum Environment {
    Filesystem {
        host_path: PathBuf,
        environment_path: PathBuf,
    },

    Hostname(String),
    DomainName(String),

    Procfs,

    Stdin,
    Stdout,
    Stderr,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Debug)]
pub enum Network {
    InternetV4,
    InternetV6,
    PrivateV4(Ipv4Network),
    PrivateV6(Ipv6Network),
}

impl Specification {
    pub fn pipes(&self) -> (Vec<&str>, Vec<&str>) {
        let mut read = Vec::new();
        let mut write = Vec::new();

        for entry in self.entrypoints.values() {
            if let Trigger::Pipe(s) = &entry.trigger {
                read.push(s.as_str());
            }

            for arg in &entry.args {
                if let Arg::Pipe(p) = arg {
                    match p {
                        Pipe::Rx(s) => read.push(s.as_str()),
                        Pipe::Tx(s) => write.push(s.as_str()),
                    }
                }
            }
        }

        debug!("read pipes: {:?}", &read);
        debug!("write pipes: {:?}", &write);
        (read, write)
    }

    pub fn sockets(&self) -> (Vec<&str>, Vec<&str>) {
        let mut read = Vec::new();
        let mut write = Vec::new();

        for entry in self.entrypoints.values() {
            if let Trigger::FileSocket(s) = &entry.trigger {
                read.push(s.as_str());
            }

            for arg in &entry.args {
                if let Arg::FileSocket(p) = arg {
                    match p {
                        FileSocket::Rx(s) => read.push(s.as_str()),
                        FileSocket::Tx(s) => write.push(s.as_str()),
                    }
                }
            }
        }

        debug!("read sockets: {:?}", &read);
        debug!("write sockets: {:?}", &write);
        (read, write)
    }

    pub fn validate(&self) -> Result<()> {
        // validate pipes match
        let (read, write) = self.pipes();
        let mut read_set = HashSet::with_capacity(read.len());

        for pipe in read {
            if !read_set.insert(pipe) {
                return Err(Error::BadPipe(pipe.to_string()));
            }
        }

        let mut write_set = HashSet::with_capacity(write.len());
        for pipe in write {
            if !write_set.insert(pipe) {
                return Err(Error::BadPipe(pipe.to_string()));
            }
        }

        for pipe in read_set {
            if !write_set.remove(pipe) {
                return Err(Error::BadPipe(pipe.to_string()));
            }
        }

        if let Some(pipe) = write_set.into_iter().next() {
            return Err(Error::BadPipe(pipe.to_string()));
        }

        // validate sockets match
        let (read, write) = self.sockets();
        let mut read_set = HashSet::with_capacity(read.len());

        for socket in read {
            if !read_set.insert(socket) {
                return Err(Error::BadFileSocket(socket.to_string()));
            }
        }

        let mut write_set = HashSet::with_capacity(write.len());
        for socket in write {
            write_set.insert(socket);
        }

        for socket in &read_set {
            if !write_set.contains(socket) {
                return Err(Error::BadFileSocket(socket.to_string()));
            }
        }

        if let Some(socket) = (&write_set - &read_set).into_iter().next() {
            return Err(Error::BadFileSocket(socket.to_string()));
        }

        // validate trigger arguments make sense
        for entrypoint in self.entrypoints.values() {
            if entrypoint.args.contains(&Arg::Trigger) {
                match entrypoint.trigger {
                    Trigger::Pipe(_) => {}
                    Trigger::FileSocket(_) => {}
                    _ => return Err(Error::BadTriggerArgument),
                }
            }
        }

        Ok(())
    }
}
