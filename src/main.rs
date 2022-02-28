use log::{debug, error, info};

mod clone;
mod error;
mod specification;

use clone::{clone3, CloneArgs, CloneFlags};
use error::Error;
use specification::{Arg, Pipe, Specification, Trigger};

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd};

use clap::{App, AppSettings};
use nix::unistd::{self, Pid};

fn main() {
    std::process::exit(match run() {
        Ok(_) => {
            info!("launched successfully");
            exitcode::OK
        }
        Err(e) => {
            error!("error: {}", e);
            -1
        }
    })
}

fn run() -> Result<(), Error> {
    // process arguments
    let matches = App::new("clone-shim")
        .version(env!("GIT_HASH"))
        .author("Jake Hillion <jake@hillion.co.uk>")
        .about("Launch a multi entrypoint app, cloning as requested by an external specification or the ELF.")
        .arg(clap::Arg::new("spec").long("specification").short('s').help("Provide the specification as an external JSON file.").takes_value(true))
        .setting(AppSettings::TrailingVarArg)
        .arg(clap::Arg::new("verbose").long("verbose").short('v').help("Use verbose logging.").takes_value(false))
        .arg(clap::Arg::new("binary").index(1).help("Binary and arguments to launch with the shim").required(true).multiple_values(true))
        .get_matches();

    let (binary, trailing) = {
        let mut argv = matches.values_of("binary").unwrap();

        let binary = argv.next().unwrap();
        let trailing: Vec<&str> = argv.collect();

        (binary, trailing)
    };

    // setup logging
    let env = env_logger::Env::new().filter_or(
        "LOG",
        if matches.is_present("verbose") {
            "debug"
        } else {
            "warn"
        },
    );
    env_logger::init_from_env(env);

    // parse the specification
    let spec: Specification = if let Some(m) = matches.value_of("spec") {
        if m.ends_with(".json") {
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

    // create all the pipes
    let (pipes, _) = spec.pipes();
    let mut read_pipes = HashMap::new();
    let mut write_pipes = HashMap::new();

    for pipe in pipes {
        info!("creating pipe pair `{}`", pipe);

        let (read, write) = unistd::pipe().map_err(|e| Error::Nix {
            msg: "pipe",
            src: e,
        })?;

        // safe to create files given the successful return of pipe(2)
        read_pipes.insert(pipe.to_string(), unsafe { File::from_raw_fd(read) });
        write_pipes.insert(pipe.to_string(), unsafe { File::from_raw_fd(write) });
    }

    // spawn all processes
    for (name, entry) in &spec.entrypoints {
        info!("spawning entrypoint `{}`", name.as_str());

        match &entry.trigger {
            Trigger::Startup => {
                if clone3(CloneArgs::new(CloneFlags::empty())).map_err(|e| Error::Nix {
                    msg: "clone3",
                    src: e,
                })? == Pid::from_raw(0)
                {
                    let mut args = Vec::new();
                    for arg in &entry.args {
                        match arg {
                            Arg::BinaryName => args.push(CString::new(binary).unwrap()),
                            Arg::Entrypoint => args.push(CString::new(name.as_str()).unwrap()),
                            Arg::Pipe(p) => args.push(match p {
                                Pipe::Rx(s) => {
                                    CString::new(read_pipes[s].as_raw_fd().to_string()).unwrap()
                                }
                                Pipe::Tx(s) => {
                                    CString::new(write_pipes[s].as_raw_fd().to_string()).unwrap()
                                }
                            }),
                            Arg::Trailing => {
                                args.extend(trailing.iter().map(|s| CString::new(*s).unwrap()))
                            }
                        }
                    }

                    unistd::execv(&CString::new(binary).unwrap(), &args).map_err(|e| {
                        Error::Nix {
                            msg: "execv",
                            src: e,
                        }
                    })?;
                }
            }
            Trigger::Pipe(_s) => {
                todo!()
            }
        }
    }

    Ok(())
}
