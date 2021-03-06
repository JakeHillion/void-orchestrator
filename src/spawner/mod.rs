use log::{debug, error, info};

mod args;
mod rpc;

use args::PreparedArgs;
use rpc::RpcHandler;

use crate::specification::{Arg, Entrypoint, Environment, Specification, Trigger};
use crate::void::VoidBuilder;
use crate::{Error, Result};
use crate::{PipePair, SocketPair};

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::path::{Path, PathBuf};

use nix::sys::signal::{kill, Signal};
use nix::sys::socket::{recvmsg, ControlMessageOwned, MsgFlags};
use nix::sys::wait::{waitid, Id, WaitPidFlag, WaitStatus};
use nix::unistd::{self, fork, ForkResult, Pid};
use nix::Error as NixError;

const BUFFER_SIZE: usize = 1024;
const MAX_FILE_DESCRIPTORS: usize = 16;

pub struct Spawner<'a> {
    pub spec: &'a Specification,
    pub binary: &'a Path,
    pub binary_args: &'a Vec<&'a str>,
    pub debug: bool,

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
                    let mut builder = VoidBuilder::new();
                    self.mount_entrypoint(&mut builder, self.binary)?;
                    self.prepare_env(&mut builder, &entrypoint.environment);

                    let args =
                        PreparedArgs::prepare_ambient_mut(self, &mut builder, &entrypoint.args)?;

                    let closure = || {
                        if self.debug {
                            Self::stop_self(name).unwrap()
                        }

                        let args = args
                            .prepare_void(self, name, &mut TriggerData::None)
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

                    let void = builder.spawn(closure)?;
                    info!("spawned entrypoint `{}` as {}", name.as_str(), void);
                }

                Trigger::Pipe(s) => {
                    let mut builder = VoidBuilder::new();
                    self.prepare_spawner(&mut builder, &entrypoint.environment, &entrypoint.args)?;

                    let pipe = self.pipes.get_mut(s).unwrap().take_read()?;
                    builder.keep_fd(&pipe);

                    let closure = || match self.pipe_trigger(pipe, entrypoint, name) {
                        Ok(()) => exitcode::OK,
                        Err(e) => {
                            error!("error in pipe_trigger: {}", e);
                            1
                        }
                    };

                    let void = builder.spawn(closure)?;
                    info!(
                        "spawned pipe trigger for entrypoint `{}` as {}",
                        name.as_str(),
                        void
                    );
                }

                Trigger::FileSocket(s) => {
                    let mut builder = VoidBuilder::new();
                    self.prepare_spawner(&mut builder, &entrypoint.environment, &entrypoint.args)?;

                    let socket = self.sockets.get_mut(s).unwrap().take_read()?;
                    builder.keep_fd(&socket);

                    let closure = || match self.file_socket_trigger(socket, entrypoint, name) {
                        Ok(()) => exitcode::OK,
                        Err(e) => {
                            error!("error in file_socket_trigger: {}", e);
                            1
                        }
                    };

                    let void = builder.spawn(closure)?;
                    info!(
                        "spawned socket trigger for entrypoint `{}` as {}",
                        name.as_str(),
                        void
                    );
                }
            }
        }

        Ok(())
    }

    fn pipe_trigger(&self, mut pipe: File, spec: &Entrypoint, name: &str) -> Result<()> {
        // put the work in a forked process that can handle signals
        Self::fork_for_trigger()?;

        let mut buf = [0_u8; BUFFER_SIZE];
        loop {
            let read_bytes = match pipe.read(&mut buf) {
                Ok(n) => Ok(n),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        return Ok(());
                    }

                    Err(e)
                }
            }?;
            if read_bytes == 0 {
                return Ok(());
            }

            debug!("triggering from pipe read");

            let mut builder = VoidBuilder::new();
            builder.mount("/entrypoint", "/entrypoint");

            self.prepare_env(&mut builder, &spec.environment);

            let args = PreparedArgs::prepare_ambient(self, &mut builder, &spec.args)?;

            let closure =
                || {
                    if self.debug {
                        Self::stop_self(name).unwrap()
                    }

                    let pipe_trigger = std::str::from_utf8(&buf[0..read_bytes]).unwrap();

                    let args = args
                        .prepare_void(self, name, &mut TriggerData::Pipe(pipe_trigger))
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

            let void = builder.spawn(closure)?;
            info!("spawned entrypoint `{}` as {}", name, void);
        }
    }

    fn file_socket_trigger(&self, socket: File, spec: &Entrypoint, name: &str) -> Result<()> {
        // put the work in a forked process that can handle signals
        Self::fork_for_trigger()?;

        let mut cmsg_buf = nix::cmsg_space!([RawFd; MAX_FILE_DESCRIPTORS]);

        loop {
            let msg = match recvmsg::<()>(
                socket.as_raw_fd(),
                &mut [],
                Some(&mut cmsg_buf),
                MsgFlags::empty(),
            ) {
                Ok(m) => Ok(m),
                Err(e) => {
                    if e == NixError::EINTR {
                        return Ok(());
                    }

                    Err(Error::Nix {
                        msg: "recvmsg",
                        src: e,
                    })
                }
            }?;

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

                        self.prepare_env(&mut builder, &spec.environment);

                        let args = PreparedArgs::prepare_ambient(self, &mut builder, &spec.args)?;

                        let closure = || {
                            if self.debug {
                                Self::stop_self(name).unwrap()
                            }

                            let args = args
                                .prepare_void(self, name, &mut TriggerData::FileSocket(fds))
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

                        let void = builder.spawn(closure)?;
                        info!("spawned entrypoint `{}` as {}", name, void);
                    }
                    _ => unimplemented!(),
                }
            }
        }
    }

    fn fork_for_trigger() -> Result<()> {
        // SAFETY: only unsafe in a multi-threaded program
        if let ForkResult::Parent { child: _pid } = unsafe { fork() }.map_err(|e| Error::Nix {
            msg: "fork",
            src: e,
        })? {
            let status = waitid(Id::All, WaitPidFlag::WEXITED).map_err(|e| Error::Nix {
                msg: "waitpid",
                src: e,
            })?;

            match status {
                WaitStatus::Exited(_pid, code) => {
                    std::process::exit(code);
                }
                WaitStatus::Signaled(pid, sig, _coredump) => {
                    debug!(
                        "trigger: forked child {} was terminated with signal {}",
                        pid, sig
                    );
                    std::process::exit(-1);
                }
                _ => unreachable!(),
            }
        }

        Ok(())
    }

    fn stop_self(name: &str) -> Result<()> {
        info!("stopping process `{}`", name);

        kill(Pid::this(), Signal::SIGSTOP).map_err(|e| Error::Nix {
            msg: "kill",
            src: e,
        })?;

        info!("process `{}` resumed", name);
        Ok(())
    }

    fn mount_entrypoint(&self, builder: &mut VoidBuilder, binary: &Path) -> Result<()> {
        let binary = PathBuf::from(binary).canonicalize()?;
        builder.mount(binary, "/entrypoint");

        Ok(())
    }

    fn forward_mounts<'b>(
        &self,
        builder: &mut VoidBuilder,
        environment: impl IntoIterator<Item = &'b Environment>,
        arguments: impl IntoIterator<Item = &'b Arg>,
    ) {
        for env in environment {
            if let Environment::Filesystem {
                host_path,
                environment_path: _,
            } = env
            {
                builder.mount(host_path, host_path);
            }
        }

        for arg in arguments {
            if let Arg::File(host_path) = arg {
                builder.mount(host_path, host_path);
            }
        }
    }

    fn forward_files<'b>(
        &self,
        builder: &mut VoidBuilder,
        arguments: impl IntoIterator<Item = &'b Arg>,
    ) {
        for arg in arguments {
            if let Arg::FileSocket(socket) = arg {
                builder.keep_fd(self.sockets.get(socket.get_name()).unwrap().write_ref());
            }
        }
    }

    fn prepare_spawner<'b>(
        &self,
        builder: &mut VoidBuilder,
        environment: impl IntoIterator<Item = &'b Environment>,
        args: impl IntoIterator<Item = &'b Arg> + Copy,
    ) -> Result<()> {
        self.mount_entrypoint(builder, self.binary)?;
        self.forward_mounts(builder, environment, args);
        self.forward_files(builder, args);

        builder.mount("/dev/null", "/dev/null");
        builder.mount("/proc", "/proc").remount_proc();

        builder.keep_fd(&1);
        builder.keep_fd(&2);

        Ok(())
    }

    fn prepare_env<'b>(
        &self,
        builder: &mut VoidBuilder,
        environment: impl IntoIterator<Item = &'b Environment>,
    ) {
        for env in environment {
            match env {
                Environment::Filesystem {
                    host_path,
                    environment_path,
                } => {
                    builder.mount(host_path, environment_path);
                }

                Environment::Hostname(name) => {
                    builder.set_hostname(name);
                }
                Environment::DomainName(name) => {
                    builder.set_domain_name(name);
                }

                Environment::Procfs => {
                    builder.mount("/proc", "/proc").remount_proc();
                }

                Environment::Stdin => {
                    builder.keep_fd(&0);
                }
                Environment::Stdout => {
                    builder.keep_fd(&1);
                }
                Environment::Stderr => {
                    builder.keep_fd(&2);
                }
            }
        }
    }
}
