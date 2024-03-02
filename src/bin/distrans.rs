use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::{arg, Parser, Subcommand};
use crypto::{digest::Digest, sha2::Sha256};
use flume::{unbounded, Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, trace, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use veilid_core::{RoutingContext, Sequencing, VeilidUpdate};

use distrans_peer::{other_err, veilid_config, Error, Fetcher, Result, Seeder};

#[derive(Parser, Debug)]
#[command(name = "distrans")]
#[command(bin_name = "distrans")]
struct Cli {
    #[arg(long, env)]
    pub state_dir: Option<String>,

    #[command(subcommand)]
    pub commands: Commands,
}

impl Cli {
    fn state_dir(&self) -> Result<String> {
        if let Some(s) = &self.state_dir {
            return Ok(s.to_owned());
        }
        match self.commands {
            Commands::Get {
                ref dht_key,
                ref root,
            } => self.state_dir_for(format!("get:{}:{}", dht_key, root)),
            Commands::Post { ref file } => self.state_dir_for(format!("post:{}", file.to_owned())),
        }
    }

    fn state_dir_for(&self, key: String) -> Result<String> {
        let mut key_digest = Sha256::new();
        key_digest.input(&key.as_bytes());
        let dir_name = key_digest.result_str();
        let data_dir =
            dirs::state_dir().ok_or(Error::Other("cannot resolve state dir".to_string()))?;
        let state_dir = data_dir
            .join("distrans")
            .join(dir_name)
            .into_os_string()
            .into_string()
            .map_err(|os| other_err(format!("{:?}", os)))?;
        debug!(state_dir);
        Ok(state_dir)
    }
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Get {
        dht_key: String,
        #[arg(default_value = ".")]
        root: String,
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
                .with_default_directive("distrans=debug".parse().unwrap())
                .from_env_lossy(),
        )
        .init();

    let cli = Cli::parse();

    run(cli).await.expect("ok");
}

async fn run(cli: Cli) -> Result<()> {
    let (routing_context, updates) =
        new_routing_context(PathBuf::from(cli.state_dir()?).as_path()).await?;
    wait_for_network(&updates).await?;

    let cancel = CancellationToken::new();
    let ctrl_c_cancel = cancel.clone();
    let complete_cancel = cancel.clone();
    let canceller = tokio::spawn(async move {
        tokio::select! {
            _ = ctrl_c_cancel.cancelled() => {
            }
            _ = tokio::signal::ctrl_c() => {
                ctrl_c_cancel.cancel();
            }
        }
    });

    match cli.commands {
        Commands::Get { dht_key, root } => {
            let fetcher =
                Fetcher::from_dht(routing_context.clone(), dht_key.as_str(), root.as_str()).await?;
            fetcher.fetch(cancel).await?;
        }
        Commands::Post { file } => {
            let seeder = Seeder::from_file(routing_context.clone(), file.as_str()).await?;
            seeder.seed(cancel, updates).await?;
        }
    }
    complete_cancel.cancel();
    if let Err(e) = canceller.await {
        warn!(err = format!("{}", e), "failed to join canceller task");
    }

    routing_context.api().shutdown().await;
    Ok(())
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

async fn wait_for_network(updates: &Receiver<VeilidUpdate>) -> Result<()> {
    // Wait for network to be up
    loop {
        let res = updates.recv_async().await;
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
