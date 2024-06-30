use std::{
    cmp::min,
    collections::HashMap,
    path::{Path, PathBuf},
};

use flume::{unbounded, Receiver, Sender};
use sha2::{Digest, Sha256};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt},
    select, spawn,
    sync::watch,
    task::JoinSet,
};

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

    pub fn empty(&self) -> Index {
        Index::empty_root(&self.root)
    }

    pub fn empty_root(root: &Path) -> Index {
        Index {
            root: root.into(),
            payload: PayloadSpec {
                digest: [0u8; 32],
                length: 0,
                pieces: vec![],
            },
            files: vec![],
        }
    }

    /// diff reconciles an index calculated over actual filesystem state with
    /// desired filesystem state, returning the file blocks that are missing or
    /// incomplete.
    pub fn diff(&self, have: &Index) -> IndexDiff {
        let mut want_file_blocks = vec![];
        let mut have_file_blocks = vec![];
        let have_files_map: HashMap<&Path, &FileSpec> =
            have.files.iter().map(|f| (f.path(), f)).collect();

        for (want_file_index, want_file) in self.files.iter().enumerate() {
            if let Some(_) = have_files_map.get(want_file.path()) {
                // Compare pieces considering the PayloadSlice
                let want_pieces = self.payload.pieces();
                let have_pieces = have.payload.pieces();
                let want_slice = want_file.contents();

                for piece_index in want_slice.starting_piece
                    ..(want_slice.starting_piece
                        + (want_slice.length / PIECE_SIZE_BYTES)
                        + if want_slice.length % PIECE_SIZE_BYTES > 0 {
                            1
                        } else {
                            0
                        })
                {
                    // TODO: Deal with payload slices that have a non-zero piece
                    // offset. This becomes a concern with multi-file indexes,
                    // where payload slices might not be aligned to piece
                    // boundaries.
                    if let Some(want_piece) = want_pieces.get(piece_index) {
                        if let Some(have_piece) = have_pieces.get(piece_index) {
                            if want_piece.digest() != have_piece.digest() {
                                let mut piece_position = 0;
                                for block_index in 0..want_piece.block_count() {
                                    let block_length =
                                        min(want_piece.length() - piece_position, BLOCK_SIZE_BYTES);
                                    want_file_blocks.push(FileBlockRef {
                                        file_index: want_file_index,
                                        piece_index,
                                        piece_offset: 0,
                                        block_index,
                                        block_length,
                                    });
                                    piece_position += block_length;
                                }
                            } else {
                                let mut piece_position = 0;
                                for block_index in 0..want_piece.block_count() {
                                    let block_length =
                                        min(want_piece.length() - piece_position, BLOCK_SIZE_BYTES);
                                    have_file_blocks.push(FileBlockRef {
                                        file_index: want_file_index,
                                        piece_index,
                                        piece_offset: 0,
                                        block_index,
                                        block_length,
                                    });
                                    piece_position += block_length;
                                }
                            }
                        } else {
                            let mut piece_position = 0;
                            for block_index in 0..want_piece.block_count() {
                                let block_length =
                                    min(want_piece.length() - piece_position, BLOCK_SIZE_BYTES);
                                want_file_blocks.push(FileBlockRef {
                                    file_index: want_file_index,
                                    piece_index,
                                    piece_offset: 0,
                                    block_index,
                                    block_length,
                                });
                                piece_position += block_length;
                            }
                        }
                    }
                }
            } else {
                // File is missing, add all blocks
                for (piece_index, piece) in self.payload.pieces().iter().enumerate() {
                    let mut piece_position = 0;
                    let piece_length = piece.length();
                    for block_index in 0..piece.block_count() {
                        let block_length = min(piece_length - piece_position, BLOCK_SIZE_BYTES);
                        want_file_blocks.push(FileBlockRef {
                            file_index: want_file_index,
                            piece_index,
                            piece_offset: 0,
                            block_index,
                            block_length,
                        });
                        piece_position += block_length;
                    }
                }
            }
        }
        IndexDiff {
            want: want_file_blocks,
            have: have_file_blocks,
        }
    }
}

pub struct IndexDiff {
    pub want: Vec<FileBlockRef>,
    pub have: Vec<FileBlockRef>,
}

pub struct FileBlockRef {
    pub file_index: usize,
    pub piece_index: usize,
    pub piece_offset: usize,
    pub block_index: usize,
    pub block_length: usize,
}

#[derive(Clone)]
pub struct Progress {
    pub length: u64,
    pub position: u64,
}

impl Default for Progress {
    fn default() -> Self {
        Progress {
            length: 0u64,
            position: 0u64,
        }
    }
}

pub struct Indexer {
    root_dir: PathBuf,
    files: Vec<PathBuf>,

    digest_progress_tx: watch::Sender<Progress>,
    index_progress_tx: watch::Sender<Progress>,
}

impl Default for Indexer {
    fn default() -> Self {
        let (digest_progress_tx, _) = watch::channel(Progress::default());
        let (index_progress_tx, _) = watch::channel(Progress::default());
        Self {
            root_dir: Default::default(),
            files: Default::default(),
            digest_progress_tx,
            index_progress_tx,
        }
    }
}

impl Indexer {
    /// from_file is used to index a complete local file on disk.
    pub async fn from_file(file: PathBuf) -> Result<Indexer> {
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

        let (digest_progress_tx, _) = watch::channel(Progress::default());
        let (index_progress_tx, _) = watch::channel(Progress::default());

        Ok(Indexer {
            root_dir: root_dir.to_path_buf(),
            files: vec![resolved_file],
            digest_progress_tx,
            index_progress_tx,
        })
    }

    pub async fn from_wanted(want: &Index) -> Result<Indexer> {
        if want.files.is_empty() {
            return Ok(Indexer::default());
        }
        if want.files.len() != 1 {
            unimplemented!("number of files > 1");
        }

        let file_path = want.root.join(want.files[0].path());
        let file_len = want.files[0].contents().length() as u64;
        if let Ok(_) = async {
            // Truncate an existing file to the wanted file length
            let fh = OpenOptions::new()
                .write(true)
                .append(true)
                .open(&file_path)
                .await?;
            if fh.metadata().await?.len() > file_len {
                fh.set_len(file_len).await?;
            }
            Ok::<(), Error>(())
        }
        .await
        {
            Indexer::from_file(file_path).await
        } else {
            Ok(Indexer::default())
        }
    }

    pub fn subscribe_digest_progress(&self) -> watch::Receiver<Progress> {
        self.digest_progress_tx.subscribe()
    }

    pub fn subscribe_index_progress(&self) -> watch::Receiver<Progress> {
        self.index_progress_tx.subscribe()
    }

    pub async fn index(&self) -> Result<Index> {
        if self.files.is_empty() {
            self.index_progress_tx.send_modify(|p| {
                p.length = 0;
                p.position = 0;
            });
            self.digest_progress_tx.send_modify(|p| {
                p.length = 0;
                p.position = 0;
            });
            return Ok(Index::empty_root(&self.root_dir));
        }
        if self.files.len() != 1 {
            unimplemented!("number of files > 1");
        }
        let resolved_file = self.files[0].to_owned();
        let payload = self.index_spec(&resolved_file).await?;
        let length = payload.length;

        // files is the file given
        Ok(Index {
            root: self.root_dir.to_owned().into(),
            payload,
            files: vec![FileSpec {
                path: resolved_file
                    .strip_prefix(&self.root_dir)
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

    async fn index_spec(&self, file: impl AsRef<Path>) -> Result<PayloadSpec> {
        let mut fh = File::open(file.as_ref()).await?;
        let file_meta = fh.metadata().await?;
        let mut payload = PayloadSpec::default();

        let (task_tx, task_rx) = unbounded::<Option<ScanTask>>();
        let (result_tx, result_rx) = unbounded::<ScanResult>();
        let mut scanners = JoinSet::new();

        let n_tasks = file_meta.len() as usize / INDEX_BUFFER_SIZE
            + if file_meta.len() as usize % INDEX_BUFFER_SIZE > 0 {
                1
            } else {
                0
            };

        for scan_index in 0..n_tasks {
            let scan_task = ScanTask {
                offset: scan_index * INDEX_BUFFER_SIZE,
                fh: File::open(file.as_ref()).await?,
            };
            task_tx
                .send_async(Some(scan_task))
                .await
                .map_err(other_err)?;
        }
        task_tx.send_async(None).await.map_err(other_err)?;

        for _ in 0..num_cpus::get() {
            scanners.spawn(Self::scan(
                task_tx.clone(),
                task_rx.clone(),
                result_tx.clone(),
            ));
        }

        let digest_progress_tx = self.digest_progress_tx.clone();
        let file_len = file_meta.len();
        let digest_task = spawn(async move {
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

                    digest_progress_tx.send_modify(|p| {
                        p.length = file_len;
                        p.position += rd as u64;
                    });
                }
                if total_rd == 0 {
                    break;
                }
                payload_digest.update(&buf[..total_rd]);
            }
            Ok::<[u8; 32], Error>(payload_digest.finalize().into())
        });

        let mut scan_results: Vec<ScanResult> = vec![];
        loop {
            select! {
                recv_result = result_rx.recv_async() => {
                    let scan_result = recv_result.map_err(other_err)?;
                    self.index_progress_tx.send_modify(|p| {
                        p.length = file_meta.len() as u64;
                        p.position += scan_result.piece.length as u64;
                    });
                    scan_results.push(scan_result);
                }
                joined = scanners.join_next() => {
                    match joined {
                        None => {
                            if result_rx.is_empty() {
                                break;
                            }
                        }
                        Some(Err(e)) => return Err(other_err(e)),
                        Some(Ok(Err(e))) => return Err(other_err(e)),
                        _ => continue,
                    }
                }
            }
        }

        // Task and result channels should be empty. Dropping them defensively
        // to force any unexpected receive still running to error noisily.
        drop(task_tx);
        drop(result_tx);

        scan_results.sort_by_key(|r| r.piece_index);
        payload.pieces = scan_results.drain(..).map(|r| r.piece).collect();
        if payload.pieces.len() > 0 {
            payload.length = (payload.pieces.len() - 1) * PIECE_SIZE_BYTES;
            payload.length += payload.pieces[payload.pieces.len() - 1].length;
        }

        let payload_digest = digest_task.await.map_err(other_err)??;
        payload.digest = payload_digest;
        Ok(payload)
    }

    async fn scan(
        task_tx: Sender<Option<ScanTask>>,
        task_rx: Receiver<Option<ScanTask>>,
        result_tx: Sender<ScanResult>,
    ) -> Result<()> {
        loop {
            match task_rx.recv_async().await {
                Ok(None) => {
                    // Create a tombstones on channels to wake the next receiver.
                    task_tx.send_async(None).await.map_err(other_err)?;
                    return Ok(());
                }
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
                        result_tx.send_async(scan_result).await.map_err(other_err)?;
                        offset += PIECE_SIZE_BYTES;
                    }
                }
                Err(e) => return Err(other_err(e)),
            };
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
mod from_file_tests {
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

        let indexer = Indexer::from_file(tempf.path().into())
            .await
            .expect("Index::from_file");
        let index = indexer.index().await.expect("index");

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

        let indexer = Indexer::from_file(tempf.path().into())
            .await
            .expect("Index::from_file");
        let index = indexer.index().await.expect("index");

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

        let indexer = Indexer::from_file(tempf.path().into())
            .await
            .expect("Index::from_file");
        let index = indexer.index().await.expect("index");

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

#[cfg(test)]
mod want_have_tests {
    use super::*;
    use std::path::PathBuf;

    // Helper function to create an Index with a single file
    fn create_index(root: PathBuf, file_path: PathBuf, piece_digests: Vec<[u8; 32]>) -> Index {
        let pieces: Vec<PayloadPiece> = piece_digests
            .into_iter()
            .map(|digest| PayloadPiece {
                digest,
                length: PIECE_SIZE_BYTES,
            })
            .collect();
        let payload = PayloadSpec {
            digest: [0; 32], // Placeholder, not used in this test
            length: pieces.len() * PIECE_SIZE_BYTES,
            pieces,
        };
        let file_spec = FileSpec {
            path: file_path,
            contents: PayloadSlice {
                starting_piece: 0,
                piece_offset: 0,
                length: payload.length,
            },
        };
        Index {
            root,
            payload,
            files: vec![file_spec],
        }
    }

    fn create_index_partial_block(
        root: PathBuf,
        file_path: PathBuf,
        piece_digests: Vec<[u8; 32]>,
    ) -> Index {
        let pieces: Vec<PayloadPiece> = piece_digests
            .into_iter()
            .map(|digest| PayloadPiece {
                digest,
                length: PIECE_SIZE_BYTES,
            })
            .collect();
        assert!(!pieces.is_empty());
        let payload = PayloadSpec {
            digest: [0; 32], // Placeholder, not used in this test
            // Same number of pieces, but the last block is short a byte.
            length: ((pieces.len() - 1) * PIECE_SIZE_BYTES) + PIECE_SIZE_BYTES - 1,
            pieces,
        };
        let file_spec = FileSpec {
            path: file_path,
            contents: PayloadSlice {
                starting_piece: 0,
                piece_offset: 0,
                length: payload.length,
            },
        };
        Index {
            root,
            payload,
            files: vec![file_spec],
        }
    }

    #[test]
    fn test_identical_files() {
        let root = PathBuf::from("/root");
        let file_path = PathBuf::from("file.txt");
        let piece_digests = vec![[1; 32], [2; 32]];

        let want_index = create_index(root.clone(), file_path.clone(), piece_digests.clone());
        let have_index = create_index(root, file_path, piece_digests);

        let diff = want_index.diff(&have_index);
        assert_eq!(diff.want.len(), 0);
        assert_eq!(diff.have.len(), 2 * PIECE_SIZE_BLOCKS);
    }

    #[test]
    fn test_identical_files_partial_block() {
        let root = PathBuf::from("/root");
        let file_path = PathBuf::from("file.txt");
        let piece_digests = vec![[1; 32], [2; 32]];

        let want_index =
            create_index_partial_block(root.clone(), file_path.clone(), piece_digests.clone());
        let have_index = create_index_partial_block(root, file_path, piece_digests);

        let diff = want_index.diff(&have_index);
        assert_eq!(diff.want.len(), 0);
        assert_eq!(diff.have.len(), 2 * PIECE_SIZE_BLOCKS);
    }

    #[test]
    fn test_have_index_empty() {
        let root = PathBuf::from("/root");
        let file_path = PathBuf::from("file.txt");
        let piece_digests = vec![[1; 32], [2; 32]];

        let want_index = create_index(root.clone(), file_path.clone(), piece_digests.clone());
        let have_index: Index = Index {
            root,
            payload: PayloadSpec {
                digest: [0; 32],
                length: 0,
                pieces: vec![],
            },
            files: vec![],
        };

        let diff = want_index.diff(&have_index);
        assert_eq!(diff.want.len(), 2 * PIECE_SIZE_BLOCKS); // Two pieces worth of blocks should be missing
        assert_eq!(diff.have.len(), 0)
    }

    #[test]
    fn test_have_index_partial_contents() {
        let root = PathBuf::from("/root");
        let file_path = PathBuf::from("file.txt");
        let want_piece_digests = vec![[1; 32], [2; 32], [3; 32]];
        let have_piece_digests = vec![[1; 32], [2; 32]];

        let want_index = create_index(root.clone(), file_path.clone(), want_piece_digests);
        let have_index = create_index(root, file_path, have_piece_digests);

        let diff = want_index.diff(&have_index);
        assert_eq!(diff.want.len(), 1 * PIECE_SIZE_BLOCKS); // One piece worth of blocks should be missing
        assert_eq!(diff.have.len(), 2 * PIECE_SIZE_BLOCKS);
    }
}
