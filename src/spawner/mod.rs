use log::{debug, error, info};

mod args;

use args::PreparedArgs;

use crate::specification::{Entrypoint, Environment, Specification, Trigger};
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
use nix::unistd::{self, Pid};

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

                    let binary = PathBuf::from(self.binary).canonicalize()?;
                    builder.mount(binary, "/entrypoint");

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
                    let pipe = self.pipes.get_mut(s).unwrap().take_read()?;
                    let binary = PathBuf::from(self.binary).canonicalize()?;

                    let mut builder = VoidBuilder::new();
                    builder.mount(binary, "/entrypoint");
                    builder.keep_fd(&pipe);

                    self.prepare_env(&mut builder, &entrypoint.environment);

                    for env in &entrypoint.environment {
                        if let Environment::Filesystem {
                            host_path,
                            environment_path: _,
                        } = env
                        {
                            builder.mount(host_path, host_path);
                        }
                    }

                    let closure = || match self.pipe_trigger(pipe, entrypoint, name) {
                        Ok(()) => std::process::exit(exitcode::OK),
                        Err(e) => {
                            error!("error in pipe_trigger: {}", e);
                            std::process::exit(1)
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
                    let socket = self.sockets.get_mut(s).unwrap().take_read()?;
                    let binary = PathBuf::from(self.binary).canonicalize()?;

                    let mut builder = VoidBuilder::new();
                    builder.mount(binary, "/entrypoint");
                    builder.keep_fd(&socket);

                    for env in &entrypoint.environment {
                        if let Environment::Filesystem {
                            host_path,
                            environment_path: _,
                        } = env
                        {
                            builder.mount(host_path, host_path);
                        }
                    }

                    let closure = || match self.file_socket_trigger(socket, entrypoint, name) {
                        Ok(()) => std::process::exit(exitcode::OK),
                        Err(e) => {
                            error!("error in file_socket_trigger: {}", e);
                            std::process::exit(1)
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
        let mut buf = [0_u8; BUFFER_SIZE];
        loop {
            let read_bytes = pipe.read(&mut buf)?;
            if read_bytes == 0 {
                return Ok(());
            }

            debug!("triggering from pipe read");

            let mut builder = VoidBuilder::new();
            builder.mount("/entrypoint", "/entrypoint");

            self.prepare_env(&mut builder, &spec.environment);

            let args = PreparedArgs::prepare_ambient(&mut builder, &spec.args)?;

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
        let mut cmsg_buf = nix::cmsg_space!([RawFd; MAX_FILE_DESCRIPTORS]);

        loop {
            let msg = recvmsg(
                socket.as_raw_fd(),
                &[],
                Some(&mut cmsg_buf),
                MsgFlags::empty(),
            )
            .map_err(|e| Error::Nix {
                msg: "recvmsg",
                src: e,
            })?;

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

                        let args = PreparedArgs::prepare_ambient(&mut builder, &spec.args)?;

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

    fn stop_self(name: &str) -> Result<()> {
        let pid = Pid::this();
        info!("stopping process `{}`", name);

        kill(pid, Signal::SIGSTOP).map_err(|e| Error::Nix {
            msg: "kill",
            src: e,
        })?;

        info!("process `{}` resumed", name);
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
            }
        }
    }
}
