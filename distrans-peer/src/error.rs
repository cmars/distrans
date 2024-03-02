use std::{array::TryFromSliceError, io, num::TryFromIntError};

use veilid_core::{Target, VeilidAPIError};

use crate::proto;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io error: {0}")]
    IO(#[from] io::Error),
    #[error("veilid api error: {0}")]
    VeilidAPI(#[from] VeilidAPIError),
    #[error("utf-8 encoding error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("fileindex error: {0}")]
    FileIndex(#[from] distrans_fileindex::Error),
    #[error("proto error: {0}")]
    Proto(#[from] proto::Error),
    #[error("{0}")]
    IntOverflow(#[from] TryFromIntError),
    #[error("{0}")]
    SliceSize(#[from] TryFromSliceError),
    #[error("not ready")]
    NotReady,
    #[error("other: {0}")]
    Other(String),
    #[error("route changed")]
    RouteChanged{target: Target},
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn other_err<T: ToString>(e: T) -> Error {
    Error::Other(e.to_string())
}
