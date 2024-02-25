use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::{arg, Parser, Subcommand};
use flume::{unbounded, Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{info, trace, warn};

use distrans_peer::{veilid_config, Error, Fetcher, Result, Seeder};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use veilid_core::{RoutingContext, Sequencing, VeilidUpdate};

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
        new_routing_context(PathBuf::from(cli.state_dir).as_path()).await?;
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