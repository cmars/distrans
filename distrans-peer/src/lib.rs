mod error;
mod fetcher;
mod peer;
mod proto;
mod seeder;
pub mod veilid_config;

use std::{path::PathBuf, sync::Arc};

use tokio::sync::broadcast::{self, Receiver, Sender};
use veilid_core::{RoutingContext, Sequencing, VeilidUpdate};

pub use error::{Error, Result};
pub use fetcher::Fetcher;
pub use peer::{Peer, ResilientPeer, VeilidPeer};
pub use seeder::Seeder;

#[cfg(test)]
pub mod tests;

const VEILID_UPDATE_CAPACITY: usize = 1024;

pub async fn new_routing_context(
    state_dir: &str,
) -> Result<(RoutingContext, Sender<VeilidUpdate>, Receiver<VeilidUpdate>)> {
    let state_path_buf = PathBuf::from(state_dir);

    let (cb_update_tx, update_rx): (Sender<VeilidUpdate>, Receiver<VeilidUpdate>) =
        broadcast::channel(VEILID_UPDATE_CAPACITY);
    let update_tx = cb_update_tx.clone();

    // Configure Veilid core
    let update_callback = Arc::new(move |change: VeilidUpdate| {
        let _ = cb_update_tx.send(change);
    });
    let config_state_path = Arc::new(state_path_buf);
    let config_callback = Arc::new(move |key| {
        veilid_config::callback(config_state_path.to_str().unwrap().to_string(), key)
    });

    // Start Veilid API
    let api: veilid_core::VeilidAPI =
        veilid_core::api_startup(update_callback, config_callback).await?;

    let routing_context = api
        .routing_context()?
        .with_sequencing(Sequencing::EnsureOrdered)
        .with_default_safety()?;
    Ok((routing_context, update_tx, update_rx))
}
