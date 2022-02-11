use log::info;

mod clone;
mod error;

use clone::{clone3, CloneArgs, CloneFlags};
use error::Error;

use nix::unistd::Pid;

fn main() -> Result<(), Error> {
    let env = env_logger::Env::new().filter_or("LOG", "info");
    env_logger::init_from_env(env);

    info!("getting started");

    if clone3(CloneArgs::new(CloneFlags::empty())).map_err(|e| Error::Nix {
        msg: "clone3",
        src: e,
    })? != Pid::from_raw(0)
    {
        info!("hello from the child");
    } else {
        info!("hello from the parent");
    }

    Ok(())
}
