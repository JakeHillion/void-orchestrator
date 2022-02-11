use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{msg}: {src}")]
    Nix { msg: &'static str, src: nix::Error },
}
