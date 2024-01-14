use std::io;

use tracing::warn;
use veilid_core::VeilidAPIError;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io error: {0}")]
    IO(#[from] io::Error),
    #[error("veilid api error: {0}")]
    VeilidAPI(#[from] VeilidAPIError),
    #[error("utf-8 encoding error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("other: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn other_err<T: ToString>(e: T) -> Error {
    Error::Other(e.to_string())
}

pub fn warn_err<T, E: std::fmt::Debug>(
    result: std::result::Result<T, E>,
    msg: &str,
) -> std::result::Result<T, E> {
    match result {
        Ok(result) => Ok(result),
        Err(e) => {
            warn!(err = format!("{:?}", e), msg);
            Err(e)
        }
    }
}
