pub mod veilid_config;
mod error;
mod proto;

pub use proto::{decode_index, encode_index};

pub use error::{Error, Result, other_err};
