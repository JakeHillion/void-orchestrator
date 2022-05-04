use log::{error, info};

use clone_shim::{pack, run, PackArgs, RunArgs};

use std::path::Path;

use clap::{Arg, Command};

fn main() {
    // process arguments
    let matches = Command::new("clone-shim")
        .version(env!("GIT_HASH"))
        .author("Jake Hillion <jake@hillion.co.uk>")
        .about("Launch a void process application.")
        .subcommand_negates_reqs(true)
        .trailing_var_arg(true)
        .subcommand(
            Command::new("pack")
                .arg(
                    Arg::new("spec")
                        .long("specification")
                        .short('s')
                        .help("Provide the specification to pack as an external JSON file.")
                        .takes_value(true)
                        .required(true),
                )
                .arg(
                    Arg::new("binary")
                        .long("binary")
                        .short('b')
                        .help("Provide the binary to pack.")
                        .takes_value(true)
                        .required(true),
                )
                .arg(
                    Arg::new("output")
                        .long("out")
                        .short('o')
                        .help("Location of the output file")
                        .takes_value(true),
                ),
        )
        .arg(
            Arg::new("spec")
                .long("specification")
                .short('s')
                .help("Provide the specification to launch as an external JSON file.")
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

    let code = if let Some(matches) = matches.subcommand_matches("pack") {
        // execute binary packing procedure
        let args = PackArgs {
            spec: Path::new(matches.value_of("spec").expect("spec required")),
            binary: Path::new(matches.value_of("binary").expect("binary required")),
            output: matches
                .value_of("output")
                .map(Path::new)
                .unwrap_or_else(|| Path::new("a.out")),
        };

        match pack(&args) {
            Ok(_) => {
                info!("binary packed successfully");
                exitcode::OK
            }
            Err(e) => {
                error!("error packing binary: {}", e);
                1
            }
        }
    } else {
        // execute shimmed process
        let (binary, binary_args) = {
            let mut argv = matches.values_of("binary").unwrap();

            let binary = Path::new(argv.next().expect("one value is required"));
            let binary_args: Vec<&str> = argv.collect();

            (binary, binary_args)
        };

        let args = RunArgs {
            spec: matches.value_of("spec").map(Path::new),
            debug: matches.is_present("debug"),
            binary,
            binary_args,
        };

        match run(&args) {
            Ok(_) => {
                info!("launched successfully");
                exitcode::OK
            }
            Err(e) => {
                error!("error: {}", e);
                -1
            }
        }
    };

    std::process::exit(code);
}
