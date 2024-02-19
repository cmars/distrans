mod error;
mod proto;
pub mod veilid_config;

pub use proto::{decode_header, decode_index, encode_header, encode_index, Header, decode_block_request, encode_block_request, BlockRequest};

pub use error::{other_err, Error, Result};
