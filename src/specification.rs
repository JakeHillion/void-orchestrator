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
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Trigger {
    Startup,
    Pipe(String),
}

impl Default for Trigger {
    fn default() -> Self {
        Self::Startup
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
// #[serde(tag = "type")]
pub enum Arg {
    /// The binary name, or argv[0], of the original program start
    BinaryName,

    /// The name of this entrypoint
    Entrypoint,

    /// A chosen end of a named pipe
    Pipe(Pipe),

    /// A value specified by the trigger
    /// NOTE: Only valid if the trigger is of type Pipe(...)
    Trigger,

    /// A TCP Listener
    TcpListener { addr: SocketAddr },

    /// The rest of argv[1..], 0 or more arguments
    Trailing,
}

impl Arg {
    fn default_vec() -> Vec<Arg> {
        vec![Arg::BinaryName]
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
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

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Debug)]
pub enum Permission {
    Filesystem {
        host_path: PathBuf,
        final_path: PathBuf,
    },
    Network {
        network: Network,
    },
    PropagateFiles,
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
            match &entry.trigger {
                Trigger::Startup => {}
                Trigger::Pipe(s) => read.push(s.as_str()),
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

        // validate pipe trigger arguments make sense
        for entrypoint in self.entrypoints.values() {
            if entrypoint.args.contains(&Arg::Trigger) {
                match entrypoint.trigger {
                    Trigger::Pipe(_) => {}
                    _ => return Err(Error::BadTriggerArgument),
                }
            }
        }

        Ok(())
    }
}
