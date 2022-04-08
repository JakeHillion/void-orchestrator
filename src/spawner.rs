use log::{debug, error, info};

use super::specification::{Arg, Entrypoint, Pipe, Specification, Trigger};
use super::PipePair;
use crate::void::VoidBuilder;
use crate::{Error, Result};

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

use nix::unistd;

const BUFFER_SIZE: usize = 1024;

pub struct Spawner<'a> {
    pub spec: &'a Specification,
    pub pipes: HashMap<String, PipePair>,
    pub binary: &'a str,
    pub trailing: &'a Vec<&'a str>,
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
                        let args = self.prepare_args(name, &entrypoint.args, None);

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
                    let pipe = self.pipes.get_mut(s).unwrap().take_read().unwrap();
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

            let closure =
                || {
                    let pipe_trigger = std::str::from_utf8(&buf[0..read_bytes]).unwrap();
                    let args = self.prepare_args_ref(name, &spec.args, Some(pipe_trigger));

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

            let mut builder = VoidBuilder::new();
            builder.spawn(closure)?;
        }
    }

    fn prepare_args(
        &mut self,
        entrypoint: &str,
        args: &[Arg],
        pipe_trigger: Option<&str>,
    ) -> Vec<CString> {
        let mut out = Vec::new();
        for arg in args {
            match arg {
                Arg::BinaryName => out.push(CString::new(self.binary).unwrap()),
                Arg::Entrypoint => out.push(CString::new(entrypoint).unwrap()),

                Arg::Pipe(p) => out.push(match p {
                    Pipe::Rx(s) => {
                        let pipe = self.pipes.get_mut(s).unwrap().take_read().unwrap();
                        CString::new(pipe.as_raw_fd().to_string()).unwrap()
                    }
                    Pipe::Tx(s) => {
                        let pipe = self.pipes.get_mut(s).unwrap().take_write().unwrap();
                        CString::new(pipe.as_raw_fd().to_string()).unwrap()
                    }
                }),

                Arg::PipeTrigger => {
                    out.push(CString::new(pipe_trigger.as_ref().unwrap().to_string()).unwrap())
                }

                Arg::TcpListener { port: _port } => unimplemented!(),

                Arg::Trailing => {
                    out.extend(self.trailing.iter().map(|s| CString::new(*s).unwrap()))
                }
            }
        }

        out
    }

    fn prepare_args_ref(
        &self,
        entrypoint: &str,
        args: &[Arg],
        pipe_trigger: Option<&str>,
    ) -> Vec<CString> {
        let mut out = Vec::new();

        for arg in args {
            match arg {
                Arg::BinaryName => out.push(CString::new(self.binary).unwrap()),
                Arg::Entrypoint => out.push(CString::new(entrypoint).unwrap()),

                Arg::Pipe(_) => panic!("can't use pipes with an immutable reference"),

                Arg::PipeTrigger => {
                    out.push(CString::new(pipe_trigger.as_ref().unwrap().to_string()).unwrap())
                }

                Arg::TcpListener { port: _port } => unimplemented!(),

                Arg::Trailing => {
                    out.extend(self.trailing.iter().map(|s| CString::new(*s).unwrap()))
                }
            }
        }

        out
    }
}
