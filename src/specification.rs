use crate::Error;

use std::collections::{HashMap, HashSet};
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
    pub permissions: HashSet<Permissions>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum Trigger {
    Startup,
    Pipe(String),
}

impl Default for Trigger {
    fn default() -> Self {
        Self::Startup
    }
}

#[derive(Serialize, Deserialize, Debug)]
// #[serde(tag = "type")]
pub enum Arg {
    /// The binary name, or argv[0], of the original program start
    BinaryName,

    /// The name of this entrypoint
    Entrypoint,

    /// A chosen end of a named pipe
    Pipe(Pipe),

    /// The rest of argv[1..], 0 or more arguments
    Trailing,
}

impl Arg {
    fn default_vec() -> Vec<Arg> {
        vec![Arg::BinaryName]
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum Pipe {
    Rx(String),
    Tx(String),
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Debug)]
#[serde(tag = "type")]
pub enum Permissions {
    Filesystem {
        host_path: PathBuf,
        final_path: PathBuf,
    },
    Network {
        network: Network,
    },
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Debug)]
#[serde(tag = "type")]
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
                match arg {
                    Arg::BinaryName => {}
                    Arg::Entrypoint => {}
                    Arg::Pipe(p) => match p {
                        Pipe::Rx(s) => read.push(s.as_str()),
                        Pipe::Tx(s) => write.push(s.as_str()),
                    },
                    Arg::Trailing => {}
                }
            }
        }

        (read, write)
    }

    pub fn validate(&self) -> Result<(), Error> {
        // validate pipes match
        let (read, write) = self.pipes();
        let mut read_set = HashSet::with_capacity(read.len());

        for pipe in read {
            if read_set.insert(pipe) {
                return Err(Error::TooManyPipes(pipe.to_string()));
            }
        }

        let mut write_set = HashSet::with_capacity(write.len());
        for pipe in write {
            if write_set.insert(pipe) {
                return Err(Error::TooManyPipes(pipe.to_string()));
            }
        }

        for pipe in read_set {
            if !write_set.remove(pipe) {
                return Err(Error::ReadOnlyPipe(pipe.to_string()));
            }
        }

        if let Some(pipe) = write_set.into_iter().next() {
            return Err(Error::WriteOnlyPipe(pipe.to_string()));
        }

        Ok(())
    }
}
