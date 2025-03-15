use std::{cmp::min, path::Path, sync::Arc};

use tokio::sync::{broadcast, RwLock};
use tracing::{debug, trace};
use veilid_core::{
    DHTRecordDescriptor, DHTSchema, KeyPair, OperationId, RoutingContext, Target, ValueData,
    VeilidAPIError, VeilidUpdate,
};

use stigmerge_fileindex::{FileSpec, Index, PayloadPiece, PayloadSpec};

use crate::{
    proto::{BlockRequest, Decoder, Encoder, Header},
    Error, Result,
};

use super::{Peer, TypedKey};

pub struct Veilid {
    routing_context: Arc<RwLock<RoutingContext>>,

    update_tx: broadcast::Sender<VeilidUpdate>,
}

impl Veilid {
    pub async fn new(
        routing_context: RoutingContext,
        update_tx: broadcast::Sender<VeilidUpdate>,
    ) -> Result<Self> {
        Ok(Veilid {
            routing_context: Arc::new(RwLock::new(routing_context)),
            update_tx,
        })
    }

    async fn open_or_create_dht_record(
        &self,
        rc: &RoutingContext,
        header: &Header,
    ) -> Result<DHTRecordDescriptor> {
        let api = rc.api();
        let ts = api.table_store()?;
        let db = ts.open("stigmerge_payload_dht", 2).await?;
        let digest_key = header.payload_digest();
        let maybe_dht_key = db.load_json(0, digest_key.as_slice()).await?;
        let maybe_dht_owner_keypair = db.load_json(1, digest_key.as_slice()).await?;
        if let (Some(dht_key), Some(dht_owner_keypair)) = (maybe_dht_key, maybe_dht_owner_keypair) {
            return Ok(rc.open_dht_record(dht_key, dht_owner_keypair).await?);
        }
        let o_cnt = header.subkeys() + 1;
        debug!(o_cnt, "header subkeys");
        let dht_rec = rc
            .create_dht_record(DHTSchema::dflt(o_cnt)?, None, None)
            .await?;
        let dht_owner = KeyPair::new(
            dht_rec.owner().to_owned(),
            dht_rec
                .owner_secret()
                .ok_or(Error::msg("expected dht owner secret"))?
                .to_owned(),
        );
        db.store_json(0, digest_key.as_slice(), dht_rec.key())
            .await?;
        db.store_json(1, digest_key.as_slice(), &dht_owner).await?;
        Ok(dht_rec)
    }

    async fn write_header(
        &self,
        rc: &RoutingContext,
        key: &TypedKey,
        header: &Header,
    ) -> Result<()> {
        // Encode the header
        let header_bytes = header.encode().map_err(Error::internal_protocol)?;
        debug!(header_length = header_bytes.len(), "writing header");

        rc.set_dht_value(key.to_owned(), 0, header_bytes, None)
            .await?;
        Ok(())
    }

    async fn write_index_bytes(
        &self,
        rc: &RoutingContext,
        dht_key: &TypedKey,
        index_bytes: &[u8],
    ) -> Result<()> {
        let mut subkey = 1; // index starts at subkey 1 (header is subkey 0)
        let mut offset = 0;
        loop {
            if offset > index_bytes.len() {
                return Ok(());
            }
            let count = min(ValueData::MAX_LEN, index_bytes.len() - offset);
            debug!(offset, count, "writing index");
            rc.set_dht_value(
                dht_key.to_owned(),
                subkey,
                index_bytes[offset..offset + count].to_vec(),
                None,
            )
            .await?;
            subkey += 1;
            offset += ValueData::MAX_LEN;
        }
    }

    async fn read_header(&self, rc: &RoutingContext, key: &TypedKey) -> Result<Header> {
        let subkey_value = match rc.get_dht_value(key.to_owned(), 0, true).await? {
            Some(value) => value,
            None => {
                return Err(VeilidAPIError::KeyNotFound {
                    key: key.to_owned(),
                }
                .into())
            }
        };
        Ok(Header::decode(subkey_value.data()).map_err(Error::remote_protocol)?)
    }

    async fn read_index(
        &self,
        rc: &RoutingContext,
        key: &TypedKey,
        header: &Header,
        root: &Path,
    ) -> Result<Index> {
        let mut index_bytes = vec![];
        for i in 0..header.subkeys() {
            let subkey_value = match rc
                .get_dht_value(key.to_owned(), (i + 1) as u32, true)
                .await?
            {
                Some(value) => value,
                None => {
                    return Err(VeilidAPIError::KeyNotFound {
                        key: key.to_owned(),
                    }
                    .into())
                }
            };
            index_bytes.extend_from_slice(subkey_value.data());
        }
        let (payload_pieces, payload_files) =
            <(Vec<PayloadPiece>, Vec<FileSpec>)>::decode(index_bytes.as_slice())
                .map_err(Error::remote_protocol)?;
        Ok(Index::new(
            root.to_path_buf(),
            PayloadSpec::new(
                header.payload_digest(),
                header.payload_length(),
                payload_pieces,
            ),
            payload_files,
        ))
    }

    async fn release_prior_route(&self, rc: &RoutingContext, prior_route: Option<Target>) {
        match prior_route {
            Some(Target::PrivateRoute(target)) => {
                let _ = rc.api().release_private_route(target);
            }
            _ => {}
        }
    }
}

impl Clone for Veilid {
    fn clone(&self) -> Self {
        Veilid {
            routing_context: self.routing_context.clone(),
            update_tx: self.update_tx.clone(),
        }
    }
}

impl Peer for Veilid {
    fn subscribe_veilid_update(&self) -> broadcast::Receiver<VeilidUpdate> {
        self.update_tx.subscribe()
    }

    async fn reset(&mut self) -> Result<()> {
        let rc = self.routing_context.write().await;
        if let Err(e) = rc.api().detach().await {
            trace!(err = e.to_string(), "detach failed");
        }
        rc.api().attach().await?;
        Ok(())
    }

    async fn shutdown(self) -> Result<()> {
        let rc = self.routing_context.write().await;
        rc.api().shutdown().await;
        Ok(())
    }

    async fn announce(&mut self, index: &Index) -> Result<(TypedKey, Target, Header)> {
        let rc = self.routing_context.read().await;
        // Serialize index to index_bytes
        let index_bytes = index.encode().map_err(Error::internal_protocol)?;
        let (announce_route, route_data) = rc.api().new_private_route().await?;
        let header = Header::from_index(index, index_bytes.as_slice(), route_data.as_slice());
        trace!(header = format!("{:?}", header));
        let dht_rec = self.open_or_create_dht_record(&rc, &header).await?;
        let dht_key = dht_rec.key().to_owned();
        self.write_index_bytes(&rc, &dht_key, index_bytes.as_slice())
            .await?;
        self.write_header(&rc, &dht_key, &header).await?;
        Ok((dht_key, Target::PrivateRoute(announce_route), header))
    }

    async fn reannounce_route(
        &mut self,
        key: &TypedKey,
        prior_route: Option<Target>,
        _index: &Index,
        header: &Header,
    ) -> Result<(Target, Header)> {
        let rc = self.routing_context.read().await;
        self.release_prior_route(&rc, prior_route).await;
        let (announce_route, route_data) = rc.api().new_private_route().await?;
        let header = header.with_route_data(route_data);
        self.write_header(&rc, &key, &header).await?;
        Ok((Target::PrivateRoute(announce_route), header))
    }

    async fn resolve(&mut self, key: &TypedKey, root: &Path) -> Result<(Target, Header, Index)> {
        let rc = self.routing_context.read().await;
        let _ = rc.open_dht_record(key.to_owned(), None).await?;
        let header = self.read_header(&rc, key).await?;
        let index = self.read_index(&rc, key, &header, &root).await?;
        let target = rc
            .api()
            .import_remote_private_route(header.route_data().to_vec())?;
        Ok((Target::PrivateRoute(target), header, index))
    }

    async fn reresolve_route(
        &mut self,
        key: &TypedKey,
        prior_route: Option<Target>,
    ) -> Result<(Target, Header)> {
        let rc = self.routing_context.read().await;
        self.release_prior_route(&rc, prior_route).await;
        let header = self.read_header(&rc, key).await?;
        let target = rc
            .api()
            .import_remote_private_route(header.route_data().to_vec())?;
        Ok((Target::PrivateRoute(target), header))
    }

    async fn request_block(
        &mut self,
        target: Target,
        piece: usize,
        block: usize,
    ) -> Result<Vec<u8>> {
        let rc = self.routing_context.read().await;
        let block_req = BlockRequest {
            piece: piece as u32,
            block: block as u8,
        };
        let block_req_bytes = block_req.encode().map_err(Error::internal_protocol)?;
        let resp_bytes = rc.app_call(target, block_req_bytes).await?;
        Ok(resp_bytes)
    }

    async fn reply_block_contents(&mut self, call_id: OperationId, contents: &[u8]) -> Result<()> {
        let rc = self.routing_context.read().await;
        rc.api().app_call_reply(call_id, contents.to_vec()).await?;
        Ok(())
    }
}
