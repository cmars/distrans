use std::{path::{Path, PathBuf}, sync::Arc};

use clap::{arg, Parser, Subcommand};
use flume::{unbounded, Receiver, Sender};
use tracing::{info, trace};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use distrans::{other_err, veilid_config, Error, Result};
use veilid_core::{DHTRecordDescriptor, RouteId, RoutingContext, Sequencing, VeilidUpdate};

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
    Get {
        dht_key: String,
        file: String,
    },
    Post {
        file: String,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            EnvFilter::builder()
                .with_default_directive("stress=debug".parse().unwrap())
                .from_env_lossy(),
        )
        .init();

    let cli = Cli::parse();

    let mut app = App::new(PathBuf::from(cli.state_dir).as_path())
        .await
        .expect("new app");
    app.wait_for_network().await.expect("network");

    match cli.commands {
        Commands::Get{dht_key, file} => {
            todo!("get")
            // Open the DHT key
            // Fetch the packed index
            // Decode the index
            // Fetch the pieces, write the file
        }
        Commands::Post{file} => {
            todo!("post")
            // Index the file
            // Create / open a DHT record for the payload digest
            // Encode the index
            // Set the subkey values
            // Mark the leader subkey as ready
            // Answer app_calls, serve the pieces
        }
    }
}

#[derive(Clone)]
struct App {
    routing_context: RoutingContext,
    updates: Receiver<VeilidUpdate>,
    dht_record: Option<DHTRecordDescriptor>,
    route_id: Option<RouteId>,
}

impl App {
    pub async fn new(path: &Path) -> Result<App> {
        let (routing_context, updates) = Self::new_routing_context(path).await?;
        Ok(App {
            routing_context,
            updates,
            dht_record: None,
            route_id: None,
        })
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
                            "Connected"
                        );
                        break;
                    }
                    info!(
                        state = attachment.state.to_string(),
                        public_internet_ready = attachment.public_internet_ready,
                        "Waiting for network"
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
}
