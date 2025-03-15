mod error;
mod fetcher;
mod peer;
mod proto;
mod seeder;
mod tracker;
pub mod veilid_config;

use std::sync::Arc;

use tokio::sync::broadcast::{self, Receiver, Sender};
use veilid_core::{RoutingContext, VeilidUpdate};

pub use error::{Error, NodeState, Result, Unexpected};
pub use fetcher::Fetcher;
pub(crate) use peer::{reset_backoff, retry_backoff};
pub use peer::{reset_with_backoff, Observable, Peer, PeerState, Veilid};
pub use seeder::Seeder;

#[cfg(test)]
pub mod tests;

const VEILID_UPDATE_CAPACITY: usize = 1024;

pub async fn new_routing_context(
    state_dir: &str,
    ns: Option<String>,
) -> Result<(RoutingContext, Sender<VeilidUpdate>, Receiver<VeilidUpdate>)> {
    let (cb_update_tx, update_rx): (Sender<VeilidUpdate>, Receiver<VeilidUpdate>) =
        broadcast::channel(VEILID_UPDATE_CAPACITY);
    let update_tx = cb_update_tx.clone();

    // Configure Veilid core
    let update_callback = Arc::new(move |change: VeilidUpdate| {
        let _ = cb_update_tx.send(change);
    });
    let config_state_path = Arc::new(state_dir.to_owned());
    let config_ns = Arc::new(ns.to_owned());
    let config_callback = Arc::new(move |key| {
        veilid_config::callback((*config_state_path).to_owned(), (*config_ns).clone(), key)
    });

    // Start Veilid API
    let api: veilid_core::VeilidAPI =
        veilid_core::api_startup(update_callback, config_callback).await?;
    api.attach().await?;

    let routing_context = api.routing_context()?;
    Ok((routing_context, update_tx, update_rx))
}
