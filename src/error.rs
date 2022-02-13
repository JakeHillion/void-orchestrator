use std::io;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{msg}: {src}")]
    Nix { msg: &'static str, src: nix::Error },

    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("bad specification type: only .json files are supported")]
    BadSpecType,

    #[error("too many pipes: a pipe must have one reader and one writer: {0}")]
    TooManyPipes(String),

    #[error("read only pipe: a pipe must have one reader and one writer: {0}")]
    ReadOnlyPipe(String),

    #[error("write only pipe: a pipe must have one reader and one writer: {0}")]
    WriteOnlyPipe(String),
}
