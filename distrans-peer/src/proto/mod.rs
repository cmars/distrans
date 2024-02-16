mod distrans_capnp;

use std::{array::TryFromSliceError, path::PathBuf, str::Utf8Error};

use capnp::{
    message::{self, ReaderOptions},
    serialize,
};
use distrans_fileindex::{FileSpec, Index, PayloadPiece, PayloadSlice, PayloadSpec};
use veilid_core::{FromStr, ValueData};

use self::distrans_capnp::index;

const MAX_RECORD_DATA_SIZE: usize = 1_048_576;
const MAX_INDEX_BYTES: usize = MAX_RECORD_DATA_SIZE - ValueData::MAX_LEN;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Capnp(#[from] capnp::Error),
    #[error("{0}")]
    NotInSchema(#[from] capnp::NotInSchema),
    #[error("{0}")]
    InternalDigestSlice(#[from] TryFromSliceError),
    #[error("index too large: {0} bytes")]
    IndexTooLarge(usize),
    #[error("{0}")]
    EncodePath(PathBuf),
    #[error("{0}")]
    DecodePath(#[from] Utf8Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn encode_index(idx: &Index) -> Result<Vec<u8>> {
    let mut builder = message::Builder::new_default();
    let mut index_builder = builder.get_root::<index::Builder>()?;
    let mut payload_builder = index_builder.reborrow().init_payload();

    // Encode the payload
    let mut payload_digest_builder = payload_builder.reborrow().init_digest();
    let payload_digest = idx.payload().digest();
    payload_digest_builder.set_p0(u64::from_be_bytes(payload_digest[0..8].try_into()?));
    payload_digest_builder.set_p1(u64::from_be_bytes(payload_digest[8..16].try_into()?));
    payload_digest_builder.set_p2(u64::from_be_bytes(payload_digest[16..24].try_into()?));
    payload_digest_builder.set_p3(u64::from_be_bytes(payload_digest[24..32].try_into()?));
    payload_builder.set_length(idx.payload().length() as u64);

    // Encode the pieces
    let mut pieces_builder = index_builder
        .reborrow()
        .init_pieces(idx.payload().pieces().len() as u32);
    for (i, idx_piece) in idx.payload().pieces().iter().enumerate() {
        let mut piece_builder = pieces_builder.reborrow().get(i as u32);
        let mut piece_digest_builder = piece_builder.reborrow().init_digest();
        let piece_digest = idx_piece.digest();
        piece_digest_builder.set_p0(u64::from_be_bytes(piece_digest[0..8].try_into()?));
        piece_digest_builder.set_p1(u64::from_be_bytes(piece_digest[8..16].try_into()?));
        piece_digest_builder.set_p2(u64::from_be_bytes(piece_digest[16..24].try_into()?));
        piece_digest_builder.set_p3(u64::from_be_bytes(piece_digest[24..32].try_into()?));
        piece_builder.set_length(idx_piece.length() as u32);
    }

    // Encode the files
    let mut files_builder = index_builder
        .reborrow()
        .init_files(idx.files().len() as u32);
    for (i, idx_file) in idx.files().iter().enumerate() {
        let mut file_builder = files_builder.reborrow().get(i as u32);
        let mut slice_builder = file_builder.reborrow().init_contents();
        slice_builder.set_starting_piece(idx_file.contents().starting_piece() as u32);
        slice_builder.set_piece_offset(idx_file.contents().piece_offset() as u32);
        slice_builder.set_length(idx_file.contents().length() as u64);
        file_builder.set_path(
            idx_file
                .path()
                .as_os_str()
                .to_str()
                .ok_or(Error::EncodePath(idx_file.path().to_owned()))?
                .into(),
        );
    }

    let message = serialize::write_message_segments_to_words(&builder);
    if message.len() > MAX_INDEX_BYTES {
        return Err(Error::IndexTooLarge(message.len()));
    }
    Ok(message)
}

pub fn decode_index(root: PathBuf, buf: &[u8]) -> Result<Index> {
    let reader = serialize::read_message(buf, ReaderOptions::new())?;
    let index_reader = reader.get_root::<index::Reader>()?;
    let payload_reader = index_reader.get_payload()?;
    let pieces_reader = index_reader.get_pieces()?;
    let files_reader = index_reader.get_files()?;

    let mut idx_pieces = vec![];
    for piece in pieces_reader.iter() {
        let mut piece_digest = [0u8; 32];
        let piece_digest_reader = piece.get_digest()?;
        piece_digest[0..8].clone_from_slice(&piece_digest_reader.get_p0().to_be_bytes()[..]);
        piece_digest[8..16].clone_from_slice(&piece_digest_reader.get_p1().to_be_bytes()[..]);
        piece_digest[16..24].clone_from_slice(&piece_digest_reader.get_p2().to_be_bytes()[..]);
        piece_digest[24..32].clone_from_slice(&piece_digest_reader.get_p3().to_be_bytes()[..]);
        idx_pieces.push(PayloadPiece::new(piece_digest, piece.get_length() as usize));
    }

    let mut idx_files = vec![];
    for file in files_reader.iter() {
        let payload_slice_reader = file.get_contents()?;
        idx_files.push(FileSpec::new(
            PathBuf::from_str(file.get_path()?.to_str()?)
                .map_err(|e| Error::Other(format!("{:?}", e)))?,
            PayloadSlice::new(
                payload_slice_reader.get_starting_piece() as usize,
                payload_slice_reader.get_piece_offset() as usize,
                payload_slice_reader.get_length() as usize,
            ),
        ));
    }

    let mut payload_digest = [0u8; 32];
    let payload_digest_reader = payload_reader.get_digest()?;
    payload_digest[0..8].clone_from_slice(&payload_digest_reader.get_p0().to_be_bytes()[..]);
    payload_digest[8..16].clone_from_slice(&payload_digest_reader.get_p1().to_be_bytes()[..]);
    payload_digest[16..24].clone_from_slice(&payload_digest_reader.get_p2().to_be_bytes()[..]);
    payload_digest[24..32].clone_from_slice(&payload_digest_reader.get_p3().to_be_bytes()[..]);

    let idx = Index::new(
        root,
        PayloadSpec::new(
            payload_digest,
            payload_reader.get_length() as usize,
            idx_pieces,
        ),
        idx_files,
    );
    Ok(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use tempfile::NamedTempFile;

    fn temp_file(pattern: u8, count: usize) -> NamedTempFile {
        let mut tempf = NamedTempFile::new().expect("temp file");
        let contents = vec![pattern; count];
        tempf.write(contents.as_slice()).expect("write temp file");
        tempf
    }

    #[tokio::test]
    async fn round_trip_index() {
        let tempf = temp_file(b'@', 4194304);
        let idx = Index::from_file(tempf.path().to_owned())
            .await
            .expect("index file");
        let message = encode_index(&idx).expect("encode index");
        let idx2 = decode_index(
            tempf.path().parent().unwrap().to_owned(),
            message.as_slice(),
        )
        .expect("decode index");
        assert_eq!(idx, idx2);
    }
}