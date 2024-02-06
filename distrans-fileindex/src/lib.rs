use std::path::{Path, PathBuf};

use crypto::digest::Digest;
use crypto::sha2::Sha256;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

mod error;

use crate::error::{other_err, Error, Result};

const BLOCK_SIZE_BYTES: usize = 32768;
const PIECE_SIZE_BLOCKS: usize = 32; // 32 * 32KB blocks = 1MB

pub struct Index {
    root: PathBuf,
    payload: PayloadSpec,
    files: Vec<FileSpec>,
}

impl Index {
    /// from_file is used to index a complete local file on disk.
    pub fn from_file(file: PathBuf) -> Result<Index> {
        match file.try_exists() {
            Ok(true) => {}
            Ok(false) => {
                return Err(error::Error::IO(std::io::Error::from(
                    std::io::ErrorKind::NotFound,
                )))
            }
            Err(e) => return Err(Error::IO(e)),
        }

        // root is directory containing file
        let resolved_file = file.canonicalize()?;
        let root_dir = resolved_file
            .parent()
            .ok_or(other_err("cannot resolve parent directory"))?;

        // payload is built from the given file

        // files is the file given

        todo!()
    }

    /// from_files is used to index a local subdirectory tree of files on disk.
    /// Empty folders are ignored; only files are indexed.
    pub fn from_files(root: PathBuf) -> Index {
        todo!()
    }

    /// from_specs is used to build an index of a file from a remote
    /// specification. This constructor is used to create an index to build the
    /// local files from an incomplete or missing download.
    pub fn from_specs(root: PathBuf, payload: PayloadSpec, files: Vec<FileSpec>) -> Index {
        Index {
            root,
            files,
            payload,
        }
    }
}

pub struct FileSpec {
    /// File name.
    path: PathBuf,

    /// File contents in payload.
    contents: PayloadSlice,
}

pub struct PayloadSlice {
    /// Starting piece where the slice begins.
    starting_piece: usize,

    /// Offset from the beginning of the starting piece where the slice starts.
    piece_offset: usize,

    /// Length of the slice. This can span multiple slices.
    length: usize,
}

pub struct PayloadSpec {
    /// SHA256 digest of the complete payload.
    digest: [u8; 32],

    /// Length of the complete payload.
    length: usize,

    /// Pieces in the file.
    pieces: Vec<PayloadPiece>,
}

impl PayloadSpec {
    fn new() -> PayloadSpec {
        PayloadSpec {
            digest: [0u8; 32],
            length: 0,
            pieces: vec![],
        }
    }

    pub async fn from_file(file: impl AsRef<Path>) -> Result<PayloadSpec> {
        let mut fh = File::open(file).await?;
        let mut buf = [0u8; BLOCK_SIZE_BYTES];
        let mut payload = PayloadSpec::new();
        let mut payload_digest = Sha256::new();
        loop {
            let mut piece = PayloadPiece {
                digest: [0u8; 32],
                length: 0,
            };
            let mut piece_digest = Sha256::new();
            for i in 0..PIECE_SIZE_BLOCKS {
                let rd = fh.read(&mut buf[..]).await?;
                if rd == 0 {
                    break;
                }
                payload.length += rd;
                piece.length += rd;
                payload_digest.input(&buf[..rd]);
                piece_digest.input(&buf[..rd]);
            }
            if piece.length == 0 {
                payload_digest.result(&mut payload.digest[..]);
                return Ok(payload);
            }
            piece_digest.result(&mut piece.digest[..]);
            payload.pieces.push(piece)
        }
    }
}

pub struct PayloadPiece {
    /// SHA256 digest of the complete piece.
    digest: [u8; 32],

    /// Length of the piece.
    /// May be < BLOCK_SIZE_BYTES * PIECE_SIZE_BLOCKS if the last piece.
    length: usize,
}

pub struct PayloadBlock {
    /// Length of the block.
    /// May be < BLOCK_SIZE_BYTES if the last block.
    length: usize,

    /// Contents of the block.
    data: Vec<u8>,
}

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
