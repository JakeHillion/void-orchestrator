use log::{error, info};

use clone_shim::run;

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
