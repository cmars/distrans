use std::cmp::min;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use distrans_fileindex::{Index, BLOCK_SIZE_BYTES, PIECE_SIZE_BLOCKS};
use flume::{unbounded, Receiver, Sender};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use veilid_core::{CryptoKey, CryptoTyped, FromStr, RoutingContext, Target, TypedKey};

use crate::proto::{decode_header, decode_index, encode_block_request, BlockRequest, Header};
use crate::{other_err, Error, Result};

#[derive(Clone)]
pub struct Fetcher {
    routing_context: RoutingContext,
    header: Header,
    index: Index,
    dht_key: CryptoTyped<CryptoKey>,
}

impl Fetcher {
    pub async fn from_dht(
        routing_context: RoutingContext,
        dht_key_str: &str,
        root: &str,
    ) -> Result<Fetcher> {
        let dht_key = TypedKey::from_str(dht_key_str)?;
        routing_context
            .open_dht_record(dht_key.clone(), None)
            .await?;

        let header = Self::read_header(&routing_context, &dht_key).await?;
        debug!(header = format!("{:?}", header));

        let root_path_buf = PathBuf::from_str(root).map_err(other_err)?;
        let index = Self::read_index(&routing_context, &dht_key, &header, &root_path_buf).await?;
        debug!(index = format!("{:?}", index));

        Ok(Fetcher {
            routing_context,
            header,
            index,
            dht_key,
        })
    }

    async fn read_header(
        routing_context: &RoutingContext,
        dht_key: &CryptoTyped<CryptoKey>,
    ) -> Result<Header> {
        let subkey_value = match routing_context
            .get_dht_value(dht_key.to_owned(), 0, true)
            .await?
        {
            Some(value) => value,
            None => return Err(Error::NotReady),
        };
        Ok(decode_header(subkey_value.data())?)
    }

    async fn read_index(
        routing_context: &RoutingContext,
        dht_key: &CryptoTyped<CryptoKey>,
        header: &Header,
        root: &Path,
    ) -> Result<Index> {
        let mut index_bytes = vec![];
        for i in 0..header.subkeys() {
            let subkey_value = match routing_context
                .get_dht_value(dht_key.to_owned(), (i + 1) as u32, true)
                .await?
            {
                Some(value) => value,
                None => return Err(Error::NotReady),
            };
            index_bytes.extend_from_slice(subkey_value.data());
        }
        Ok(decode_index(
            root.to_path_buf(),
            header,
            index_bytes.as_slice(),
        )?)
    }

    pub async fn fetch(self, cancel: CancellationToken) -> Result<()> {
        let (sender, receiver) = unbounded();

        let index = self.index.clone();
        let mut tasks = JoinSet::new();
        tasks.spawn(Self::enqueue_blocks(cancel.clone(), sender.clone(), index));
        tasks.spawn(self.clone().fetch_blocks(cancel.clone(), sender, receiver));
        let mut result = Ok(());
        while let Some(join_res) = tasks.join_next().await {
            match join_res {
                Ok(res) => {
                    if let Err(e) = res {
                        warn!(err = format!("{}", e));
                        result = Err(e);
                    }
                }
                Err(e) => result = Err(other_err(e)),
            }
        }

        if let Err(e) = self
            .routing_context
            .close_dht_record(self.dht_key.to_owned())
            .await
        {
            warn!(err = format!("{}", e), "failed to close DHT record");
        }
        result
    }

    async fn fetch_blocks(
        self,
        cancel: CancellationToken,
        sender: Sender<FileBlockFetch>,
        receiver: Receiver<FileBlockFetch>,
    ) -> Result<()> {
        let mut fh_map: HashMap<usize, File> = HashMap::new();
        let target = self
            .routing_context
            .api()
            .import_remote_private_route(self.header.route_data().to_vec())?;
        let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                recv_fetch = receiver.recv_async() => {
                    let fetch = match recv_fetch {
                        Ok(fetch) => fetch,
                        Err(e) => {
                            // See https://docs.rs/flume/latest/flume/struct.Receiver.html#method.recv_async
                            // An error here means "channel is closed" effectively. Makes me miss Go.
                            // This is practically impossible given we have a sender in scope here for retries...
                            debug!(err = format!("{}", e), "all senders have been dropped");
                            return Ok(())
                        }
                    };
                    debug!(fetch = format!("{:?}", fetch));
                    let fetch_result: Result<()> = async {
                        let fh = match fh_map.get_mut(&fetch.file_index) {
                            Some(fh) => fh,
                            None => {
                                let path = self.index.root().join(self.index.files()[fetch.file_index].path());
                                let fh = File::options().write(true).truncate(false).create(true).open(path).await?;
                                fh_map.insert(fetch.file_index, fh);
                                fh_map.get_mut(&fetch.file_index).unwrap()
                            }
                        };
                        let mut block = self.request_block(
                            Target::PrivateRoute(target.to_owned()),
                            fetch.piece_index,
                            fetch.block_index,
                        ).await?;
                        let block_end = min(block.len(), BLOCK_SIZE_BYTES);
                        fh.write_all(block[fetch.piece_offset..block_end].as_mut()).await?;
                        // TODO: update piece completion; we could verify
                        // concurrently with fetching other pieces, reject &
                        // requeue bad ones...
                        Ok(())
                    }.await;
                    if let Err(e) = fetch_result {
                        warn!(err = format!("{}", e), "fetch block failed, queued for retry");
                        sender.send(fetch).map_err(other_err)?;
                    }
                }
                _ = heartbeat.tick() => {
                    if receiver.is_empty() {
                        debug!("all blocks have been fetched");
                        // TODO: verify pieces!!!
                        return Ok(())
                    }
                }
                _ = cancel.cancelled() => {
                    return Err(other_err("cancelled"))
                }
            }
        }
    }

    async fn request_block(&self, target: Target, piece: usize, block: usize) -> Result<Vec<u8>> {
        let block_req = BlockRequest {
            piece: piece as u32,
            block: block as u8,
        };
        let block_req_bytes = encode_block_request(&block_req)?;
        let resp_bytes = self
            .routing_context
            .app_call(target, block_req_bytes)
            .await?;
        Ok(resp_bytes)
    }

    async fn enqueue_blocks(
        cancel: CancellationToken,
        sender: Sender<FileBlockFetch>,
        index: Index,
    ) -> Result<()> {
        for (file_index, file_spec) in index.files().iter().enumerate() {
            let mut piece_index = file_spec.contents().starting_piece();
            let mut piece_offset = file_spec.contents().piece_offset();
            let mut block_index = piece_offset / BLOCK_SIZE_BYTES;
            let mut pos = 0;
            while pos < file_spec.contents().length() {
                if cancel.is_cancelled() {
                    return Err(other_err("cancelled"));
                }
                sender
                    .send(FileBlockFetch {
                        file_index,
                        piece_index,
                        piece_offset,
                        block_index,
                    })
                    .map_err(other_err)?;
                pos += BLOCK_SIZE_BYTES - piece_offset;
                piece_offset = 0;
                block_index += 1;
                if block_index == PIECE_SIZE_BLOCKS {
                    piece_index += 1;
                    block_index = 0;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct FileBlockFetch {
    file_index: usize,
    piece_index: usize,
    piece_offset: usize,
    block_index: usize,
}