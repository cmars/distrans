use std::sync::Arc;

use distrans::{Result, veilid_config, Error};
use flume::{Receiver, Sender, unbounded};
use tracing::{info, trace, warn};
use veilid_core::{RoutingContext, VeilidUpdate, Sequencing};

#[tokio::main]
async fn main() {
    let (rc, recv) = new_routing_context(".randgen").unwrap();
    println!("Hello, world!");
}

struct App {
    routing_context: RoutingContext,
    updates: Receiver<VeilidUpdate>,
}

impl App {
    async fn new_routing_context(
        state_path: &str,
    ) -> Result<(RoutingContext, Receiver<VeilidUpdate>)> {
        // Veilid API state channel
        let (node_sender, updates): (Sender<VeilidUpdate>, Receiver<VeilidUpdate>) = unbounded();

        // Start up Veilid core
        let update_callback = Arc::new(move |change: VeilidUpdate| {
            let _ = node_sender.send(change);
        });
        let config_state_path = Arc::new(state_path.to_owned());
        let config_callback =
            Arc::new(move |key| veilid_config::callback(config_state_path.to_string(), key));

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

        // TODO: Set DHT state
        loop {
            match self.push().await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    warn!(err = format!("{:?}", e), "failed to push status");
                }
            }
        }
    }
}
