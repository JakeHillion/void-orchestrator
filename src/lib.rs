use log::{debug, info};

pub mod clone;
mod error;
mod spawner;
mod specification;
mod void;

use error::Error;
use spawner::Spawner;
use specification::Specification;

use std::collections::HashMap;
use std::fs::File;
use std::os::unix::io::FromRawFd;

use clap::{App, AppSettings};
use nix::fcntl::OFlag;
use nix::unistd::{self};

pub fn run() -> Result<(), Error> {
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
    let pipes = create_pipes(pipes)?;

    // spawn all processes
    Spawner {
        spec: &spec,
        pipes,
        binary,
        trailing: &trailing,
    }
    .spawn()
}

pub struct PipePair {
    read: Option<File>,
    write: Option<File>,
}

impl PipePair {
    fn take_read(&mut self) -> File {
        self.read
            .take()
            .expect("read pipes should only be used once")
    }

    fn take_write(&mut self) -> File {
        self.write
            .take()
            .expect("write pipes should only be used once")
    }
}

fn create_pipes(names: Vec<&str>) -> Result<HashMap<String, PipePair>, Error> {
    let mut pipes = HashMap::new();

    for pipe in names {
        info!("creating pipe pair `{}`", pipe);

        let (read, write) = unistd::pipe2(OFlag::O_DIRECT).map_err(|e| Error::Nix {
            msg: "pipe2",
            src: e,
        })?;

        // safe to create files given the successful return of pipe(2)
        pipes.insert(
            pipe.to_string(),
            PipePair {
                read: Some(unsafe { File::from_raw_fd(read) }),
                write: Some(unsafe { File::from_raw_fd(write) }),
            },
        );
    }

    Ok(pipes)
}
