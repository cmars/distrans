use std::{
    cmp::min,
    path::{Path, PathBuf},
};

use flume::{unbounded, Receiver, Sender};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::{fs::File, task::JoinSet};

pub use crate::error::{other_err, Error, Result};

mod error;

pub const BLOCK_SIZE_BYTES: usize = 32768;
pub const PIECE_SIZE_BLOCKS: usize = 32; // 32 * 32KB blocks = 1MB
pub const PIECE_SIZE_BYTES: usize = PIECE_SIZE_BLOCKS * BLOCK_SIZE_BYTES;
const INDEX_BUFFER_SIZE: usize = 67108864; // 64MB

#[derive(Debug, PartialEq, Clone)]
pub struct Index {
    root: PathBuf,
    payload: PayloadSpec,
    files: Vec<FileSpec>,
}

impl Index {
    pub fn new(root: PathBuf, payload: PayloadSpec, files: Vec<FileSpec>) -> Index {
        Index {
            root,
            payload,
            files,
        }
    }

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

#[derive(Debug, PartialEq, Clone)]
pub struct FileSpec {
    /// File name.
    path: PathBuf,

    /// File contents in payload.
    contents: PayloadSlice,
}

impl FileSpec {
    pub fn new(path: PathBuf, contents: PayloadSlice) -> FileSpec {
        FileSpec { path, contents }
    }

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
    pub fn new(starting_piece: usize, piece_offset: usize, length: usize) -> PayloadSlice {
        PayloadSlice {
            starting_piece,
            piece_offset,
            length,
        }
    }

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

#[derive(Debug, PartialEq, Clone)]
pub struct PayloadSpec {
    /// SHA256 digest of the complete payload.
    digest: [u8; 32],

    /// Length of the complete payload.
    length: usize,

    /// Pieces in the file.
    pieces: Vec<PayloadPiece>,
}

impl PayloadSpec {
    pub fn new(digest: [u8; 32], length: usize, pieces: Vec<PayloadPiece>) -> PayloadSpec {
        PayloadSpec {
            digest,
            length,
            pieces,
        }
    }

    pub fn digest(&self) -> &[u8] {
        &self.digest[..]
    }

    pub fn length(&self) -> usize {
        self.length
    }

    pub fn pieces(&self) -> &Vec<PayloadPiece> {
        &self.pieces
    }

    pub async fn from_file(file: impl AsRef<Path>) -> Result<PayloadSpec> {
        let mut fh = File::open(file.as_ref()).await?;
        let file_meta = fh.metadata().await?;
        let mut payload = PayloadSpec::default();

        let (task_sender, task_receiver) = unbounded::<Option<ScanTask>>();
        let (result_sender, result_receiver) = unbounded::<ScanResult>();
        let mut scanners = JoinSet::new();

        let n_tasks = file_meta.len() as usize / INDEX_BUFFER_SIZE
            + if file_meta.len() as usize % INDEX_BUFFER_SIZE > 0 {
                1
            } else {
                0
            };
        for _ in 0..n_tasks {
            scanners.spawn(Self::scan(task_receiver.clone(), result_sender.clone()));
        }

        for scan_index in 0..n_tasks {
            let scan_task = ScanTask {
                offset: scan_index * INDEX_BUFFER_SIZE,
                fh: File::open(file.as_ref()).await?,
            };
            task_sender
                .send_async(Some(scan_task))
                .await
                .map_err(other_err)?;
        }

        let mut buf = vec![0; INDEX_BUFFER_SIZE];
        let mut payload_digest = Sha256::new();
        loop {
            let mut total_rd = 0;
            while total_rd < INDEX_BUFFER_SIZE {
                let rd = fh.read(&mut buf[total_rd..INDEX_BUFFER_SIZE]).await?;
                if rd == 0 {
                    break;
                }
                total_rd += rd;
            }
            if total_rd == 0 {
                break;
            }
            payload_digest.update(&buf[..total_rd]);
        }
        drop(buf);
        payload.digest = payload_digest.finalize().into();

        loop {
            task_sender.send_async(None).await.map_err(other_err)?;
            match scanners.join_next().await {
                None => break,
                Some(Err(e)) => return Err(other_err(e)),
                Some(Ok(Err(e))) => return Err(other_err(e)),
                Some(Ok(Ok(()))) => {}
            }
        }
        drop(task_sender);

        let mut scan_result: Vec<ScanResult> = result_receiver.drain().collect();
        scan_result.sort_by_key(|r| r.piece_index);
        payload.pieces = scan_result.drain(..).map(|r| r.piece).collect();
        if payload.pieces.len() > 0 {
            payload.length = (payload.pieces.len() - 1) * PIECE_SIZE_BYTES;
            payload.length += payload.pieces[payload.pieces.len() - 1].length;
        }
        //payload_digest.result(&mut payload.digest[..]);
        Ok(payload)
    }

    async fn scan(receiver: Receiver<Option<ScanTask>>, sender: Sender<ScanResult>) -> Result<()> {
        loop {
            match receiver.recv_async().await {
                Ok(None) => return Ok(()),
                Ok(Some(mut scan_task)) => {
                    scan_task
                        .fh
                        .seek(std::io::SeekFrom::Start(
                            scan_task.offset.try_into().map_err(other_err)?,
                        ))
                        .await?;
                    let mut total_rd = 0;
                    let mut buf = vec![0u8; INDEX_BUFFER_SIZE];
                    while total_rd < INDEX_BUFFER_SIZE {
                        let rd = scan_task
                            .fh
                            .read(&mut buf[total_rd..INDEX_BUFFER_SIZE])
                            .await?;
                        if rd == 0 {
                            break;
                        }
                        total_rd += rd;
                    }
                    if total_rd == 0 {
                        return Ok(());
                    }

                    let mut offset = 0;
                    while offset < total_rd {
                        let piece_index = (scan_task.offset + offset) / PIECE_SIZE_BYTES;
                        let piece_length = min(PIECE_SIZE_BYTES, total_rd - offset);
                        let mut piece_digest = Sha256::new();
                        piece_digest.update(&buf[offset..offset + piece_length]);
                        let mut piece = PayloadPiece {
                            digest: [0u8; 32],
                            length: piece_length,
                        };
                        piece.digest = piece_digest.finalize().into();
                        let scan_result = ScanResult { piece_index, piece };
                        sender.send_async(scan_result).await.map_err(other_err)?;
                        offset += PIECE_SIZE_BYTES;
                    }
                }
                Err(e) => return Err(other_err(e)),
            };
        }
    }
}

#[derive(Debug)]
struct ScanTask {
    offset: usize,
    fh: File,
}

#[derive(Debug)]
struct ScanResult {
    piece_index: usize,
    piece: PayloadPiece,
}

impl Default for PayloadSpec {
    fn default() -> PayloadSpec {
        PayloadSpec {
            digest: [0u8; 32],
            length: 0,
            pieces: vec![],
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct PayloadPiece {
    /// SHA256 digest of the complete piece.
    digest: [u8; 32],

    /// Length of the piece.
    /// May be < BLOCK_SIZE_BYTES * PIECE_SIZE_BLOCKS if the last piece.
    length: usize,
}

impl PayloadPiece {
    pub fn new(digest: [u8; 32], length: usize) -> PayloadPiece {
        PayloadPiece { digest, length }
    }

    pub fn digest(&self) -> &[u8] {
        return &self.digest[..];
    }

    pub fn length(&self) -> usize {
        return self.length;
    }

    pub fn block_count(&self) -> usize {
        self.length / BLOCK_SIZE_BYTES
            + if self.length % BLOCK_SIZE_BYTES > 0 {
                1
            } else {
                0
            }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use hex_literal::hex;
    use tempfile::NamedTempFile;

    use super::*;

    pub fn temp_file(pattern: u8, count: usize) -> NamedTempFile {
        let mut tempf = NamedTempFile::new().expect("temp file");
        let contents = vec![pattern; count];
        tempf.write(contents.as_slice()).expect("write temp file");
        tempf
    }

    #[tokio::test]
    async fn single_file_index() {
        let tempf = temp_file(b'.', 1049600);

        let index = Index::from_file(tempf.path().into())
            .await
            .expect("Index::from_file");

        assert_eq!(
            index.root().to_owned(),
            tempf.path().parent().unwrap().to_owned()
        );

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
    }

    #[tokio::test]
    async fn empty_file_index() {
        let tempf = temp_file(b'.', 0);

        let index = Index::from_file(tempf.path().into())
            .await
            .expect("Index::from_file");

        assert_eq!(
            index.root().to_owned(),
            tempf.path().parent().unwrap().to_owned()
        );

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
                length: 0,
            }
        );
        assert_eq!(index.payload().pieces().len(), 0);
        assert_eq!(
            index.payload().digest(),
            hex!("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
    }

    #[tokio::test]
    async fn large_file_index() {
        let tempf = temp_file(b'.', 134217728);

        let index = Index::from_file(tempf.path().into())
            .await
            .expect("Index::from_file");

        assert_eq!(
            index.root().to_owned(),
            tempf.path().parent().unwrap().to_owned()
        );

        // Index files
        assert_eq!(index.files().len(), 1);
        assert_eq!(
            index.files()[0].path().to_owned(),
            tempf.path().file_name().unwrap().to_owned()
        );
        assert_eq!(index.payload().pieces().len(), 128);
        assert_eq!(
            index.files()[0].contents(),
            PayloadSlice {
                piece_offset: 0,
                starting_piece: 0,
                length: 134217728,
            }
        );
    }
}
