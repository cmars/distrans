use std::{
    cmp::min,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::{arg, Parser, Subcommand};
use distrans_fileindex::{Index, BLOCK_SIZE_BYTES, PIECE_SIZE_BLOCKS};
use flume::{unbounded, Receiver, Sender};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    select,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, trace, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use veilid_core::{
    DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, FromStr, KeyPair, RouteId, RoutingContext,
    Sequencing, TypedKey, ValueData, VeilidAPIError, VeilidUpdate,
};

use distrans::{
    decode_block_request, encode_header, encode_index, other_err, veilid_config, Error, Header,
    Result,
};

#[derive(Parser, Debug)]
#[command(name = "distrans")]
#[command(bin_name = "distrans")]
struct Cli {
    #[arg(long, env)]
    pub state_dir: String,

    #[command(subcommand)]
    pub commands: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Get { dht_key: String, file: String },
    Post { file: String },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            EnvFilter::builder()
                .with_default_directive("distrans=debug".parse().unwrap())
                .from_env_lossy(),
        )
        .init();

    let cli = Cli::parse();

    let mut app = App::new(PathBuf::from(cli.state_dir).as_path())
        .await
        .expect("new app");
    app.wait_for_network().await.expect("network");

    match cli.commands {
        Commands::Get { dht_key, file } => {
            app.get(&dht_key, &file).await.expect("get");
        }
        Commands::Post { file } => {
            app.post(&file).await.expect("post");
        }
    }
}

#[derive(Clone)]
struct App {
    routing_context: RoutingContext,
    updates: Receiver<VeilidUpdate>,
    dht_record: Option<DHTRecordDescriptor>,
    header: Option<Header>,
    index: Option<Index>,
    route_id: Option<RouteId>,
    cancel: CancellationToken,
}

impl App {
    pub async fn new(path: &Path) -> Result<App> {
        let (routing_context, updates) = Self::new_routing_context(path).await?;
        Ok(App {
            routing_context,
            updates,
            dht_record: None,
            header: None,
            index: None,
            route_id: None,
            cancel: CancellationToken::new(),
        })
    }

    pub fn cancel(&self) -> CancellationToken {
        self.cancel.clone()
    }

    fn dht_record(&self) -> Result<&DHTRecordDescriptor> {
        self.dht_record.as_ref().ok_or(Error::NotReady)
    }

    fn header(&self) -> Result<&Header> {
        self.header.as_ref().ok_or(Error::NotReady)
    }

    fn index(&self) -> Result<&Index> {
        self.index.as_ref().ok_or(Error::NotReady)
    }

    pub async fn get(&mut self, dht_key_str: &str, file: &str) -> Result<()> {
        let dht_key = TypedKey::from_str(dht_key_str)?;
        let dht_rec = self
            .routing_context
            .open_dht_record(dht_key.clone(), None)
            .await?;
        let index = self.get_index(dht_rec)?;
        self.fetch_from_index(index, file.into())?;
        if let Err(e) = self.routing_context.close_dht_record(dht_key).await {
            warn!(err = format!("{}", e), "failed to close dht record");
        }
        Ok(())
    }

    pub async fn post(&mut self, file: &str) -> Result<()> {
        // Index the file
        let index = Index::from_file(file.into()).await?;

        // Encode the index
        let index_bytes = encode_index(&index)?;
        let index_length = index_bytes.len();
        debug!(index_length);

        let (route_id, route_data) = self.routing_context.api().new_private_route().await?;

        let header = Header::new(
            index.payload().digest().try_into()?,
            index.payload().length(),
            ((index_length / 32768) + if (index_length % 32768) > 0 { 1 } else { 0 }).try_into()?,
            route_data.as_slice(),
        );

        // Create / open a DHT record for the payload digest
        let dht_rec = self.open_or_create_dht_record(&header).await?;
        info!(dht_key = format!("{}", dht_rec.key()));

        self.dht_record = Some(dht_rec);
        self.header = Some(header);
        self.index = Some(index);
        self.route_id = Some(route_id);

        // Writing the header last, ensures we have a complete index written before "announcing".
        self.write_index_bytes(index_bytes.as_slice()).await?;
        self.write_header().await?;

        debug!("dht updated, handling requests now");
        self.handle_post_requests(file.into()).await
    }

    async fn new_routing_context(
        state_path: &Path,
    ) -> Result<(RoutingContext, Receiver<VeilidUpdate>)> {
        // Veilid API state channel
        let (node_sender, updates): (Sender<VeilidUpdate>, Receiver<VeilidUpdate>) = unbounded();

        // Start up Veilid core
        let update_callback = Arc::new(move |change: VeilidUpdate| {
            let _ = node_sender.send(change);
        });
        let config_state_path = Arc::new(state_path.to_owned());
        let config_callback = Arc::new(move |key| {
            veilid_config::callback(config_state_path.to_str().unwrap().to_string(), key)
        });

        let api: veilid_core::VeilidAPI =
            veilid_core::api_startup(update_callback, config_callback).await?;
        api.attach().await?;

        let routing_context = api
            .routing_context()?
            .with_sequencing(Sequencing::EnsureOrdered)
            .with_default_safety()?;
        Ok((routing_context, updates))
    }

    pub async fn wait_for_network(&mut self) -> Result<()> {
        // Wait for network to be up
        loop {
            let res = self.updates.recv_async().await;
            match res {
                Ok(VeilidUpdate::Attachment(attachment)) => {
                    if attachment.public_internet_ready {
                        info!(
                            state = attachment.state.to_string(),
                            public_internet_ready = attachment.public_internet_ready,
                            "connected"
                        );
                        break;
                    }
                    info!(
                        state = attachment.state.to_string(),
                        public_internet_ready = attachment.public_internet_ready,
                        "waiting for network"
                    );
                }
                Ok(u) => {
                    trace!(update = format!("{:?}", u));
                }
                Err(e) => {
                    return Err(Error::Other(e.to_string()));
                }
            };
        }
        Ok(())
    }

    async fn open_or_create_dht_record(&mut self, header: &Header) -> Result<DHTRecordDescriptor> {
        let ts = self.routing_context.api().table_store()?;
        let db = ts.open("distrans_payload_dht", 2).await?;
        let digest_key = header.payload_digest();
        let maybe_dht_key = db.load_json(0, digest_key.as_slice()).await?;
        let maybe_dht_owner_keypair = db.load_json(1, digest_key.as_slice()).await?;
        if let (Some(dht_key), Some(dht_owner_keypair)) = (maybe_dht_key, maybe_dht_owner_keypair) {
            return Ok(self
                .routing_context
                .open_dht_record(dht_key, dht_owner_keypair)
                .await?);
        }
        let o_cnt = header.subkeys() + 1;
        let dht_rec = self
            .routing_context
            .create_dht_record(DHTSchema::DFLT(DHTSchemaDFLT { o_cnt }), None)
            .await?;
        let dht_owner = KeyPair::new(
            dht_rec.owner().to_owned(),
            dht_rec
                .owner_secret()
                .ok_or(other_err("expected dht owner secret"))?
                .to_owned(),
        );
        db.store_json(0, digest_key.as_slice(), dht_rec.key()).await?;
        db.store_json(1, digest_key.as_slice(), &dht_owner).await?;
        Ok(dht_rec)
    }

    async fn handle_post_requests(&mut self, path: PathBuf) -> Result<()> {
        let mut fh = File::open(path).await?;
        loop {
            select! {
                recv_update = self.updates.recv_async() => {
                    let update = match recv_update {
                        Ok(update) => update,
                        Err(e) => return Err(other_err(e)),
                    };
                    self.handle_post_update(&mut fh, update).await?;
                }
                _ = self.cancel.cancelled() => {
                    info!("cancel requested");
                    return Ok(())
                }
            }
        }
    }

    async fn handle_post_update(&mut self, fh: &mut File, update: VeilidUpdate) -> Result<()> {
        // TODO: reuse buffer across requests?
        let mut buf = [0u8; BLOCK_SIZE_BYTES];
        match update {
            VeilidUpdate::AppCall(app_call) => {
                let block_request = decode_block_request(app_call.message())?;
                fh.seek(std::io::SeekFrom::Start(
                    // TODO: wire this through Index to support multifile
                    ((block_request.piece as usize * PIECE_SIZE_BLOCKS * BLOCK_SIZE_BYTES)
                        + (block_request.block as usize * BLOCK_SIZE_BYTES))
                        as u64,
                ))
                .await?;
                let rd = fh.read(&mut buf).await?;
                // TODO: Don't block here; we could handle another request in the meantime
                self.routing_context
                    .api()
                    .app_call_reply(app_call.id(), buf[0..rd].to_vec())
                    .await?;
                Ok(())
            }
            VeilidUpdate::RouteChange(_) => {
                let (route_id, route_data) = self.routing_context.api().new_private_route().await?;
                info!(route_id = format!("{}", route_id), "route changed");
                let header = self.header()?;
                self.header = Some(header.with_route_data(route_data));
                self.route_id = Some(route_id);
                self.write_header().await?;
                Ok(())
            }
            VeilidUpdate::Shutdown => Err(Error::VeilidAPI(VeilidAPIError::Shutdown)),
            _ => Ok(()),
        }
    }

    fn get_index(&self, dht_rec: DHTRecordDescriptor) -> Result<Index> {
        todo!()
    }

    fn fetch_from_index(&self, index: Index, file: PathBuf) -> Result<()> {
        todo!()
    }

    async fn write_header(&self) -> Result<()> {
        // Encode the header
        let header = self.header()?;
        let header_bytes = encode_header(self.index()?, header.subkeys(), header.route_data())?;
        debug!(header_length = header_bytes.len(), "writing header");

        self.routing_context
            .set_dht_value(self.dht_record()?.key().to_owned(), 0, header_bytes)
            .await?;
        Ok(())
    }

    async fn write_index_bytes(&self, index_bytes: &[u8]) -> Result<()> {
        let dht_key = self.dht_record()?.key();
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
                    dht_key.to_owned(),
                    subkey,
                    index_bytes[offset..offset + count].to_vec(),
                )
                .await?;
            subkey += 1;
            offset += ValueData::MAX_LEN;
        }
    }
}
