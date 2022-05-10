use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{msg}: {src}")]
    Nix { msg: &'static str, src: nix::Error },

    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("bincode: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("elf: read: {0}")]
    ElfRead(#[from] object::read::Error),

    #[error("elf: write: {0}")]
    ElfWrite(#[from] object::write::Error),

    #[error("bad pipe specification: a pipe must have exactly one reader and one writer: {0}")]
    BadPipe(String),

    #[error("bad socket specification: a socket must have exactly one reader and one writer: {0}")]
    BadFileSocket(String),

    #[error("no specification provided")]
    NoSpecification,

    #[error("bad specification type: only json files are supported")]
    BadSpecType,

    #[error("bad trigger argument: this entrypoint is not triggered by something with arguments")]
    BadTriggerArgument,
}
