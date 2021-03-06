use log::{debug, info};

pub mod clone;
mod error;
mod spawner;
mod specification;
mod void;

use error::{Error, Result};
use spawner::Spawner;
use specification::{Environment, Specification};

use std::collections::HashMap;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::Path;

use nix::fcntl::OFlag;
use nix::sys::socket;
use nix::sys::wait::{waitid, Id, WaitPidFlag, WaitStatus};
use nix::unistd;

pub struct RunArgs<'a> {
    pub spec: Option<&'a Path>,
    pub debug: bool,
    pub daemon: bool,

    pub stdout: bool,
    pub stderr: bool,

    pub binary: &'a Path,
    pub binary_args: Vec<&'a str>,
}

pub fn run(args: &RunArgs) -> Result<i32> {
    // parse the specification
    let mut spec: Specification = if let Some(m) = args.spec {
        if m.extension().map(|e| e == "json") == Some(true) {
            let f = std::fs::File::open(m)?;
            Ok(serde_json::from_reader(f)?)
        } else {
            Err(Error::BadSpecType)
        }
    } else {
        unimplemented!("reading spec from the elf is unimplemented")
    }?;

    debug!("specification read: {:?}", &spec);
    spec.validate()?;

    if args.stdout {
        debug!("forwarding stdout");
        for entrypoint in &mut spec.entrypoints.values_mut() {
            entrypoint.environment.insert(Environment::Stdout);
        }
    }

    if args.stderr {
        debug!("forwarding stderr");
        for entrypoint in &mut spec.entrypoints.values_mut() {
            entrypoint.environment.insert(Environment::Stderr);
        }
    }

    // create all the pipes
    let (pipes, _) = spec.pipes();
    let pipes = create_pipes(pipes)?;

    let (sockets, _) = spec.sockets();
    let sockets = create_sockets(sockets)?;

    // spawn all processes
    Spawner {
        spec: &spec,
        binary: args.binary,
        binary_args: &args.binary_args,
        debug: args.debug,

        pipes,
        sockets,
    }
    .spawn()?;

    if args.daemon {
        return Ok(exitcode::OK);
    }

    info!("spawned successfully, awaiting children exiting...");
    let mut exit_code = exitcode::OK;

    loop {
        let status = match waitid(Id::All, WaitPidFlag::WEXITED) {
            Ok(v) => Ok(v),
            Err(nix::Error::ECHILD) => {
                info!("all child processes have exited, exiting...");
                break;
            }
            Err(e) => Err(Error::Nix {
                msg: "waitpid",
                src: e,
            }),
        }?;

        match status {
            WaitStatus::Exited(pid, code) => {
                if code != exitcode::OK {
                    exit_code = code;
                }
                debug!("child {} exited with code {}", pid, code);
            }
            WaitStatus::Signaled(pid, sig, _coredump) => {
                debug!("child {} was terminated with signal {}", pid, sig);
            }
            _ => unreachable!(),
        }
    }

    Ok(exit_code)
}

fn create_pipes(names: Vec<&str>) -> Result<HashMap<String, PipePair>> {
    let mut pipes = HashMap::new();
    for pipe in names {
        info!("creating pipe pair `{}`", pipe);
        pipes.insert(pipe.to_string(), PipePair::new(pipe)?);
    }

    Ok(pipes)
}

fn create_sockets(names: Vec<&str>) -> Result<HashMap<String, SocketPair>> {
    let mut sockets = HashMap::new();
    for socket in names {
        info!("creating socket pair `{}`", socket);
        sockets.insert(socket.to_string(), SocketPair::new(socket)?);
    }

    Ok(sockets)
}

pub struct PipePair {
    name: String,

    read: Option<File>,
    write: Option<File>,
}

impl PipePair {
    fn new(name: &str) -> Result<PipePair> {
        let (read, write) = unistd::pipe2(OFlag::O_DIRECT).map_err(|e| Error::Nix {
            msg: "pipe2",
            src: e,
        })?;

        Ok(PipePair {
            name: name.to_string(),
            // SAFETY: valid new fd as pipe2(2) returned successfully
            read: Some(unsafe { File::from_raw_fd(read) }),
            // SAFETY: valid new fd as pipe2(2) returned successfully
            write: Some(unsafe { File::from_raw_fd(write) }),
        })
    }

    fn take_read(&mut self) -> Result<File> {
        self.read
            .take()
            .ok_or_else(|| Error::BadPipe(self.name.to_string()))
    }

    fn take_write(&mut self) -> Result<File> {
        self.write
            .take()
            .ok_or_else(|| Error::BadPipe(self.name.to_string()))
    }
}

pub struct SocketPair {
    name: String,

    read: Option<File>,
    write: File,
}

impl SocketPair {
    fn new(name: &str) -> Result<SocketPair> {
        let (read, write) = socket::socketpair(
            socket::AddressFamily::Unix,
            socket::SockType::Datagram,
            None,
            socket::SockFlag::empty(),
        )
        .map_err(|e| Error::Nix {
            msg: "socketpair",
            src: e,
        })?;

        Ok(SocketPair {
            name: name.to_string(),
            // SAFETY: valid new fd as socketpair(2) returned successfully
            read: Some(unsafe { File::from_raw_fd(read) }),
            // SAFETY: valid new fd as socketpair(2) returned successfully
            write: unsafe { File::from_raw_fd(write) },
        })
    }

    fn take_read(&mut self) -> Result<File> {
        self.read
            .take()
            .ok_or_else(|| Error::BadFileSocket(self.name.to_string()))
    }

    fn write(&self) -> Result<File> {
        let dup_fd = nix::unistd::dup(self.write.as_raw_fd())
            .map_err(|e| Error::Nix { msg: "dup", src: e })?;

        // SAFETY: valid new fd as dup(2) returned successfully
        Ok(unsafe { File::from_raw_fd(dup_fd) })
    }

    fn write_ref(&self) -> &File {
        &self.write
    }
}
