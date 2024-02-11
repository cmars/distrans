use std::path::{Path, PathBuf};

use crypto::digest::Digest;
use crypto::sha2::Sha256;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

mod error;

use crate::error::{other_err, Error, Result};

const BLOCK_SIZE_BYTES: usize = 32768;
const PIECE_SIZE_BLOCKS: usize = 32; // 32 * 32KB blocks = 1MB

#[derive(Debug, PartialEq)]
pub struct Index {
    root: PathBuf,
    payload: PayloadSpec,
    files: Vec<FileSpec>,
}

impl Index {
    pub fn root(&self) -> &Path {
        return self.root.as_ref();
    }

    pub fn payload(&self) -> &PayloadSpec {
        return &self.payload;
    }

    pub fn files(&self) -> &Vec<FileSpec> {
        return &self.files;
    }

    /// from_file is used to index a complete local file on disk.
    pub async fn from_file(file: PathBuf) -> Result<Index> {
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
        let root_dir = &resolved_file
            .parent()
            .ok_or(other_err("cannot resolve parent directory"))?;

        // payload is built from the given file
        let payload = PayloadSpec::from_file(&resolved_file).await?;
        let length = payload.length;

        // files is the file given
        Ok(Index {
            root: root_dir.into(),
            payload,
            files: vec![FileSpec {
                path: resolved_file
                    .strip_prefix(root_dir)
                    .map_err(other_err)?
                    .to_owned(),
                contents: PayloadSlice {
                    starting_piece: 0,
                    piece_offset: 0,
                    length,
                },
            }],
        })
    }

    /// from_files is used to index a local subdirectory tree of files on disk.
    /// Empty folders are ignored; only files are indexed.
    pub fn from_files(_root: PathBuf) -> Index {
        unimplemented!()
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

#[derive(Debug, PartialEq)]
pub struct FileSpec {
    /// File name.
    path: PathBuf,

    /// File contents in payload.
    contents: PayloadSlice,
}

impl FileSpec {
    pub fn path(&self) -> &Path {
        return self.path.as_ref();
    }

    pub fn contents(&self) -> PayloadSlice {
        self.contents
    }
}

#[derive(Debug, PartialEq, Eq, Copy)]
pub struct PayloadSlice {
    /// Starting piece where the slice begins.
    starting_piece: usize,

    /// Offset from the beginning of the starting piece where the slice starts.
    piece_offset: usize,

    /// Length of the slice. This can span multiple slices.
    length: usize,
}

impl PayloadSlice {
    pub fn starting_piece(&self) -> usize {
        return self.starting_piece;
    }

    pub fn piece_offset(&self) -> usize {
        return self.piece_offset;
    }

    pub fn length(&self) -> usize {
        return self.length;
    }
}

impl Clone for PayloadSlice {
    fn clone(&self) -> Self {
        PayloadSlice {
            starting_piece: self.starting_piece,
            piece_offset: self.piece_offset,
            length: self.length,
        }
    }
}

#[derive(Debug, PartialEq)]
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

    pub fn digest(&self) -> &[u8] {
        return &self.digest[..]
    }

    pub fn length(&self) -> usize {
        return self.length
    }

    pub fn pieces(&self) -> &Vec<PayloadPiece> {
        return &self.pieces
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
            for _ in 0..PIECE_SIZE_BLOCKS {
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

#[derive(Debug, PartialEq)]
pub struct PayloadPiece {
    /// SHA256 digest of the complete piece.
    digest: [u8; 32],

    /// Length of the piece.
    /// May be < BLOCK_SIZE_BYTES * PIECE_SIZE_BLOCKS if the last piece.
    length: usize,
}

impl PayloadPiece {
    pub fn digest(&self) -> &[u8] {
        return &self.digest[..]
    }

    pub fn length(&self) -> usize {
        return self.length
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn temp_file(pattern: &[u8], count: usize) -> Result<NamedTempFile> {
        let mut tempf = NamedTempFile::new().expect("temp file");
        for _ in 0..count {
            tempf.write(pattern)?;
        }
        Ok(tempf)
    }

    #[tokio::test]
    async fn single_file_index() {
        let tempf = temp_file(b".", 1049600).expect("write temp file");

        let index = Index::from_file(tempf.path().into())
            .await
            .expect("Index::from_file");

        assert_eq!(index.root().to_owned(), tempf.path().parent().unwrap().to_owned());

        // Index files
        assert_eq!(index.files().len(), 1);
        assert_eq!(
            index.files()[0].path().to_owned(),
            tempf.path().file_name().unwrap().to_owned()
        );
        assert_eq!(
            index.files()[0].contents(),
            PayloadSlice {
                piece_offset: 0,
                starting_piece: 0,
                length: 1049600,
            }
        );

        // Index payload
        assert_eq!(
            index.payload().digest(),
            hex!("529df3a7e7acab0e3b53e7cd930faa22e62cd07a948005b1c3f7f481f32a7297")
        );
        assert_eq!(index.payload().length(), 1049600);
        assert_eq!(index.payload().pieces().len(), 2);
        assert_eq!(index.payload().pieces()[0].length(), 1048576);
        assert_eq!(
            index.payload().pieces()[0].digest(),
            hex!("153faf1f2a007097d33120bbee6944a41cb8be7643c1222f6bc6bc69ec31688f")
        );
        assert_eq!(index.payload().pieces()[1].length(), 1024);
        assert_eq!(
            index.payload().pieces()[1].digest(),
            hex!("ca33403cfcb21bae20f21507475a3525c7f4bd36bb2a7074891e3307c5fd47d5")
        );

        println!("{:?}", index);
    }
}
