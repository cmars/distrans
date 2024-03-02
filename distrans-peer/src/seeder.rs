use std::{cmp::min, path::PathBuf};

use flume::Receiver;
use path_absolutize::*;
use tokio::{fs::File, io::{AsyncReadExt, AsyncSeekExt}, select};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};
use veilid_core::{
    CryptoKey, CryptoTyped, DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, KeyPair, RoutingContext, ValueData, VeilidAPIError, VeilidUpdate
};

use distrans_fileindex::{Index, BLOCK_SIZE_BYTES, PIECE_SIZE_BLOCKS};

use crate::{other_err, proto::{decode_block_request, encode_header}, Error, Result};
use crate::proto::{encode_index, Header};

pub struct Seeder {
    routing_context: RoutingContext,
    index: Index,
    header: Header,
    dht_key: CryptoTyped<CryptoKey>,
}

impl Seeder {
    #[instrument(skip(routing_context), level = "debug", err)]
    pub async fn from_file(routing_context: RoutingContext, file: &str) -> Result<Seeder> {
        // Derive root directory
        let file_path_buf: PathBuf = PathBuf::try_from(file).map_err(other_err)?;
        let abs_file = file_path_buf.absolutize()?;

        // Build an index of the content to be shared
        info!(path = format!("{:?}", file_path_buf), "indexing file");
        let index = Index::from_file(abs_file.into()).await?;
        let index_bytes = encode_index(&index)?;

        // Initial private route
        let (_, route_data) = routing_context.api().new_private_route().await?;

        // Initial header
        let header = Header::new(
            index.payload().digest().try_into()?,
            index.payload().length(),
            ((index_bytes.len() / 32768)
                + if (index_bytes.len() % 32768) > 0 {
                    1
                } else {
                    0
                })
            .try_into()?,
            route_data.as_slice(),
        );

        // Create / open a DHT record for the payload digest
        let dht_rec = Self::open_or_create_dht_record(&routing_context, &header).await?;

        let seeder = Seeder{
            routing_context,
            index,
            header,
            dht_key: dht_rec.key().to_owned(),
        };
        seeder.announce(index_bytes.as_slice()).await?;
        Ok(seeder)
    }

    #[instrument(skip(routing_context, header), level = "trace", err)]
    async fn open_or_create_dht_record(
        routing_context: &RoutingContext,
        header: &Header,
    ) -> Result<DHTRecordDescriptor> {
        let ts = routing_context.api().table_store()?;
        let db = ts.open("distrans_payload_dht", 2).await?;
        let digest_key = header.payload_digest();
        let maybe_dht_key = db.load_json(0, digest_key.as_slice()).await?;
        let maybe_dht_owner_keypair = db.load_json(1, digest_key.as_slice()).await?;
        if let (Some(dht_key), Some(dht_owner_keypair)) = (maybe_dht_key, maybe_dht_owner_keypair) {
            return Ok(routing_context
                .open_dht_record(dht_key, dht_owner_keypair)
                .await?);
        }
        let o_cnt = header.subkeys() + 1;
        let dht_rec = routing_context
            .create_dht_record(DHTSchema::DFLT(DHTSchemaDFLT { o_cnt }), None)
            .await?;
        let dht_owner = KeyPair::new(
            dht_rec.owner().to_owned(),
            dht_rec
                .owner_secret()
                .ok_or(other_err("expected dht owner secret"))?
                .to_owned(),
        );
        db.store_json(0, digest_key.as_slice(), dht_rec.key())
            .await?;
        db.store_json(1, digest_key.as_slice(), &dht_owner).await?;
        Ok(dht_rec)
    }

    #[instrument(skip(self, index_bytes), level = "trace", err)]
    async fn announce(&self, index_bytes: &[u8]) -> Result<()> {
        // Writing the header last, ensures we have a complete index written before "announcing".
        self.write_index_bytes(index_bytes).await?;
        self.write_header().await?;
        Ok(())
    }

    async fn write_header(&self) -> Result<()> {
        // Encode the header
        let header_bytes = encode_header(&self.index, self.header.subkeys(), self.header.route_data())?;
        debug!(header_length = header_bytes.len(), "writing header");

        self.routing_context
            .set_dht_value(self.dht_key.to_owned(), 0, header_bytes)
            .await?;
        Ok(())
    }

    async fn write_index_bytes(&self, index_bytes: &[u8]) -> Result<()> {
        let mut subkey = 1; // index starts at subkey 1 (header is subkey 0)
        let mut offset = 0;
        loop {
            if offset > index_bytes.len() {
                return Ok(());
            }
            let count = min(ValueData::MAX_LEN, index_bytes.len() - offset);
            debug!(offset, count, "writing index");
            self.routing_context
                .set_dht_value(
                    self.dht_key.to_owned(),
                    subkey,
                    index_bytes[offset..offset + count].to_vec(),
                )
                .await?;
            subkey += 1;
            offset += ValueData::MAX_LEN;
        }
    }

    pub async fn seed(mut self, cancel: CancellationToken, updates: Receiver<VeilidUpdate>) -> Result<()> {
        if self.index.files().len() > 1 {
            todo!("multi-file seeding not yet supported, sorry!");
        }
        let local_single_file = self.index.root().join(self.index.files()[0].path());
        info!(dht_key = format!("{}", self.dht_key), file = format!("{:?}", local_single_file), "seeding");
        let mut fh: File = File::open(local_single_file).await?;
        let mut buf = [0u8; BLOCK_SIZE_BYTES];

        loop {
            select! {
                recv_update = updates.recv_async() => {
                    let update = match recv_update {
                        Ok(update) => update,
                        Err(e) => return Err(other_err(e)),
                    };
                    if let Err(e) = self.handle_update(&mut fh, &mut buf, update).await {
                        warn!(err = format!("{:?}", e), "failed to handle update");
                    };
                }
                _ = cancel.cancelled() => {
                    info!("seeding cancelled");
                    break
                }
            }
        }

        if let Err(e) = self.routing_context.close_dht_record(self.dht_key).await {
            warn!(err = format!("{:?}", e), "failed to close DHT record");
        }
        Ok(())
    }

    async fn handle_update(&mut self, fh: &mut File, buf: &mut [u8], update: VeilidUpdate) -> Result<()> {
        match update {
            VeilidUpdate::AppCall(app_call) => {
                let block_request = decode_block_request(app_call.message())?;
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
                self.routing_context
                    .api()
                    .app_call_reply(app_call.id(), buf[0..rd].to_vec())
                    .await?;
                Ok(())
            }
            VeilidUpdate::RouteChange(_) => {
                let (route_id, route_data) = self.routing_context.api().new_private_route().await?;
                debug!(route_id = format!("{}", route_id), "route changed");
                self.header = self.header.with_route_data(route_data);
                self.write_header().await?;
                Ok(())
            }
            VeilidUpdate::Shutdown => Err(Error::VeilidAPI(VeilidAPIError::Shutdown)),
            _ => Ok(()),
        }
    }

}