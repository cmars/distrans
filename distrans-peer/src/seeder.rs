use std::path::PathBuf;

use backoff::{backoff::Backoff, ExponentialBackoff};
use path_absolutize::*;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    select,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument};
use veilid_core::{Target, VeilidAPIError, VeilidUpdate};

use distrans_fileindex::{Index, Indexer, BLOCK_SIZE_BYTES, PIECE_SIZE_BLOCKS};

use crate::proto::Header;
use crate::{
    peer::{Peer, ShareKey},
    proto::decode_block_request,
    Error, Result,
};

pub struct Seeder<P: Peer> {
    peer: P,
    index: Index,
    share_key: ShareKey,
    target: Target,
    header: Header,
}

impl<P: Peer> Seeder<P> {
    pub async fn new(mut peer: P, index: Index) -> Result<Seeder<P>> {
        let (share_key, target, header) = peer.announce(&index).await?;
        Ok(Seeder {
            peer,
            index,
            share_key,
            target,
            header,
        })
    }

    #[deprecated = "use new(peer, index) instead"]
    #[instrument(skip(peer), level = "debug", err)]
    pub async fn from_file(peer: P, file: &str) -> Result<Seeder<P>> {
        // Derive root directory
        let file_path_buf: PathBuf = PathBuf::from(file);
        let abs_file = file_path_buf.absolutize()?;

        // Build an index of the content to be shared
        info!(path = format!("{:?}", file_path_buf), "indexing file");
        let indexer = Indexer::from_file(abs_file.into())
            .await
            .map_err(Error::index)?;
        let index = indexer.index().await.map_err(Error::index)?;
        Self::new(peer, index).await
    }

    pub fn share_key(&self) -> String {
        self.share_key.to_string()
    }

    pub fn digest(&self) -> String {
        hex::encode(self.index.payload().digest())
    }

    pub async fn seed(mut self, cancel: CancellationToken) -> Result<()> {
        if self.index.files().len() > 1 {
            todo!("multi-file seeding not yet supported, sorry!");
        }
        let local_single_file = self.index.root().join(self.index.files()[0].path());
        info!(
            share_key = format!("{}", self.share_key),
            file = format!("{:?}", local_single_file),
            "seeding"
        );

        let mut fh: File = File::open(local_single_file).await?;
        let mut buf = [0u8; BLOCK_SIZE_BYTES];

        let mut updates = self.peer.subscribe_veilid_update();

        loop {
            select! {
                recv_update = updates.recv() => {
                    let update = recv_update.map_err(Error::other)?;
                    let mut back_off = Self::reroute_backoff();
                    loop {
                        match self.handle_update(&mut fh, &mut buf, &update).await {
                            Ok(()) => break,
                            Err(e) => {
                                if Error::is_route_invalid(&e) {
                                    if let Some(delay) = back_off.next_backoff() {
                                        tokio::time::sleep(delay).await;
                                    }
                                    else {
                                        return Err(e)
                                    }
                                    let (target, header) = self.peer.reannounce_route(&self.share_key, Some(self.target), &self.index, &self.header).await?;
                                    self.target = target;
                                    self.header = header;
                                } else {
                                    return Err(e)
                                }
                            }
                        };
                    }
                }
                _ = cancel.cancelled() => {
                    info!("seeding cancelled");
                    break
                }
            }
        }
        Ok(())
    }

    fn reroute_backoff() -> ExponentialBackoff {
        ExponentialBackoff::default()
    }

    async fn handle_update(
        &mut self,
        fh: &mut File,
        buf: &mut [u8],
        update: &VeilidUpdate,
    ) -> Result<()> {
        match update {
            &VeilidUpdate::AppCall(ref app_call) => {
                let block_request =
                    decode_block_request(app_call.message()).map_err(Error::remote_protocol)?;
                // TODO: mmap would enable more concurrency here, but might not be as cross-platform?
                fh.seek(std::io::SeekFrom::Start(
                    // TODO: wire this through Index to support multifile
                    ((block_request.piece as usize * PIECE_SIZE_BLOCKS * BLOCK_SIZE_BYTES)
                        + (block_request.block as usize * BLOCK_SIZE_BYTES))
                        as u64,
                ))
                .await?;
                let rd = fh.read(buf).await?;
                // TODO: Don't block here; we could handle another request in the meantime
                self.peer
                    .reply_block_contents(app_call.id(), &buf[0..rd])
                    .await?;
                Ok(())
            }
            &VeilidUpdate::RouteChange(ref route_change) => {
                let target_route_id = match self.target {
                    Target::NodeId(_) => return Ok(()),
                    Target::PrivateRoute(ref route_id) => route_id.to_owned(),
                };
                if !route_change.dead_routes.contains(&target_route_id) {
                    return Ok(());
                }
                debug!(target = target_route_id.to_string(), "route changed");

                let (target, header) = self
                    .peer
                    .reannounce_route(
                        &self.share_key,
                        Some(self.target.to_owned()),
                        &self.index,
                        &self.header,
                    )
                    .await?;
                self.target = target;
                self.header = header;
                Ok(())
            }
            &VeilidUpdate::Shutdown => Err(Error::Fault(crate::error::Unexpected::Veilid(
                VeilidAPIError::Shutdown,
            ))),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use tokio::time::sleep;
    use veilid_core::{
        FromStr, OperationId, TypedKey, VeilidAppCall, VeilidRouteChange, VeilidStateAttachment,
        VeilidUpdate,
    };

    use crate::{
        error::Unexpected,
        proto::{encode_block_request, encode_index, BlockRequest},
        tests::{temp_file, StubPeer},
        ResilientPeer,
    };

    use super::*;

    #[tokio::test]
    async fn from_dht_ok() {
        // Temp file to seed, index it, create a stub key for target and share
        let tf = temp_file(0xa5u8, 1048576);
        let tf_path = std::env::temp_dir().join(tf.path()).to_owned();
        let announce_indexer = Indexer::from_file(tf_path.clone().into())
            .await
            .expect("index");
        let announce_index = announce_indexer.index().await.expect("index");
        let key = TypedKey::from_str("VLD0:cCHB85pEaV4bvRfywxnd2fRNBScR64UaJC8hoKzyr3M").unwrap();
        let key_internal = key.clone();

        // Track reannounce call
        let reannounce_index = announce_index.clone();
        let reannounce_calls = Arc::new(Mutex::new(0));
        let reannounce_calls_internal = reannounce_calls.clone();

        // Track reply_block_contents
        let reply_calls = Arc::new(Mutex::new(0));
        let reply_calls_internal = reply_calls.clone();

        let mut stub_peer = StubPeer::new();
        let update_tx = stub_peer.update_tx.clone();
        stub_peer.reset_result = Arc::new(Mutex::new(move || Ok(())));
        stub_peer.announce_result = Arc::new(Mutex::new(move || {
            let index_bytes = encode_index(&announce_index).expect("encode index");
            Ok((
                key_internal.clone(),
                Target::PrivateRoute(key_internal.value.clone()),
                Header::from_index(&announce_index, &index_bytes, &[0xde, 0xad, 0xbe, 0xef]),
            ))
        }));
        stub_peer.reannounce_route_result = Arc::new(Mutex::new(move || {
            let index_bytes = encode_index(&reannounce_index).expect("encode index");
            (*(reannounce_calls_internal.lock().unwrap())) += 1;
            Ok((
                Target::PrivateRoute(key_internal.value.clone()),
                Header::from_index(&reannounce_index, &index_bytes, &[0xde, 0xad, 0xbe, 0xef]),
            ))
        }));
        stub_peer.reply_block_contents_result = Arc::new(Mutex::new(move || {
            (*(reply_calls_internal.lock().unwrap())) += 1;
            Ok(())
        }));
        let rp = ResilientPeer::new(stub_peer);

        // Simulate getting connected to network, normally track_node_state
        // would set this when the node comes online.
        update_tx
            .send(VeilidUpdate::Attachment(Box::new(VeilidStateAttachment {
                state: veilid_core::AttachmentState::AttachedGood,
                public_internet_ready: true,
                local_network_ready: true,
            })))
            .expect("send veilid update");

        let seeder = Seeder::from_file(rp, tf_path.to_str().unwrap())
            .await
            .expect("from_file");
        let cancel = CancellationToken::new();
        let update_tx_internal = update_tx.clone();
        tokio::spawn(async move {
            // Simulate a request for a block
            let request_bytes = encode_block_request(&BlockRequest { piece: 0, block: 0 })
                .expect("encode block request");
            sleep(Duration::from_millis(50)).await;
            update_tx_internal
                .send(VeilidUpdate::AppCall(Box::new(VeilidAppCall::new(
                    None,
                    None,
                    request_bytes,
                    OperationId::new(42u64),
                ))))
                .expect("send app call");

            // Create a route change
            sleep(Duration::from_millis(50)).await;
            update_tx_internal
                .send(VeilidUpdate::RouteChange(Box::new(VeilidRouteChange {
                    dead_routes: vec![key_internal.value],
                    dead_remote_routes: vec![],
                })))
                .expect("send route change");

            // Shut down the node
            sleep(Duration::from_millis(50)).await;
            update_tx_internal
                .send(VeilidUpdate::Shutdown)
                .expect("shutdown");
            Ok::<(), Error>(())
        });
        let result = seeder.seed(cancel).await;
        assert!(matches!(
            result,
            Err(Error::Fault(Unexpected::Veilid(VeilidAPIError::Shutdown)))
        ));

        assert_eq!(*(reannounce_calls.lock().unwrap()), 1);
        assert_eq!(*(reply_calls.lock().unwrap()), 1);
    }
}
