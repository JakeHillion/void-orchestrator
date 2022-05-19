use log::error;

use clone_shim::{run, RunArgs};

use std::path::Path;

use clap::{Arg, Command};

fn main() {
    // process arguments
    let matches = Command::new("clone-shim")
        .version(env!("GIT_HASH"))
        .author("Jake Hillion <jake@hillion.co.uk>")
        .about("Launch a void process application.")
        .trailing_var_arg(true)
        .arg(
            Arg::new("spec")
                .long("specification")
                .short('s')
                .help("Provide the specification as an external JSON file.")
                .takes_value(true),
        )
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .short('v')
                .help("Use verbose logging.")
                .takes_value(false),
        )
        .arg(
            Arg::new("debug")
                .long("debug")
                .short('d')
                .help("Stop each spawned application process so that it can be attached to.")
                .takes_value(false),
        )
        .arg(
            Arg::new("daemon")
                .long("daemon")
                .short('D')
                .help("Detach the shim from all child processes and exit immediately.")
                .takes_value(false),
        )
        .arg(
            Arg::new("binary")
                .index(1)
                .help("Binary and arguments to launch with the shim")
                .required(true)
                .multiple_values(true),
        )
        .get_matches();

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

    // launch process
    // execute shimmed process
    std::process::exit({
        let (binary, binary_args) = {
            let mut argv = matches.values_of("binary").unwrap();

            let binary = Path::new(argv.next().expect("one value is required"));
            let binary_args: Vec<&str> = argv.collect();

            (binary, binary_args)
        };

        let args = RunArgs {
            spec: matches.value_of("spec").map(Path::new),
            debug: matches.is_present("debug"),
            daemon: matches.is_present("daemon"),
            binary,
            binary_args,
        };

        match run(&args) {
            Ok(_) => exitcode::OK,
            Err(e) => {
                error!("error: {}", e);
                -1
            }
        }
    })
}
