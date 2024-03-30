mod error;
mod fetcher;
mod proto;
mod seeder;
pub mod veilid_config;

use std::time::Duration;

use flume::Receiver;
use tokio::{select, time::sleep_until};
use tracing::{info, trace};
use veilid_core::{RoutingContext, VeilidAPIError, VeilidUpdate};

pub use error::{other_err, Error, Result};
pub use fetcher::Fetcher;
pub use seeder::Seeder;

pub async fn wait_for_network<F>(updates: &Receiver<VeilidUpdate>, on_update: F) -> Result<()>
where
    F: Fn(&VeilidUpdate) -> (),
{
    let timeout = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(60))
        .unwrap();
    loop {
        select! {
            recv_update = updates.recv_async() => {
                let update = recv_update.map_err(other_err)?;
                on_update(&update);
                match update {
                    VeilidUpdate::Attachment(attachment) => {
                        if attachment.public_internet_ready {
                            info!("connected");
                            break;
                        }
                    }
                    VeilidUpdate::Shutdown => {
                        return Err(VeilidAPIError::Shutdown.into());
                    }
                    u => {
                        trace!(update = format!("{:?}", u));
                    }
                };
            }
            _ = sleep_until(timeout) => {
                return Err(other_err("timed out waiting for network"));
            }
        }
    }
    Ok(())
}

pub async fn reconnect(
    routing_context: &RoutingContext,
    updates: &Receiver<VeilidUpdate>,
) -> Result<()> {
    routing_context.api().detach().await?;
    routing_context.api().attach().await?;
    wait_for_network(updates, |_| {}).await?;
    Ok(())
}
