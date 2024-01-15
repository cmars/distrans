use std::{path::Path, sync::Arc};

use distrans::{other_err, veilid_config, Error, Result};

use flume::{unbounded, Receiver, Sender};
use futures::future::join_all;
use tempdir::TempDir;
use tokio::{select, signal, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;
use veilid_core::{
    DHTRecordDescriptor, DHTSchemaDFLT, FromStr, RouteId, RoutingContext, Sequencing, Target,
    TypedKey, VeilidUpdate,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            EnvFilter::builder()
                .with_default_directive("randgen=debug".parse().unwrap())
                .from_env_lossy(),
        )
        .init();

    let app_dir = TempDir::new("randgen").expect("tempdir");
    let mut app = App::new(app_dir.path()).await.expect("new app");
    app.wait_for_network().await.expect("network");
    app.announce().await.expect("announce");
    let dht_key = (&app.dht_record.as_ref()).unwrap().key().to_owned();
    println!("{}", dht_key);

    let remotes = std::env::args().skip(1).collect();

    app.serve(remotes).await.expect("serve");

    app.routing_context
        .delete_dht_record(dht_key)
        .await
        .expect("delete dht record");
    info!("dht record deleted");
    app.routing_context.api().shutdown().await;
    info!("shutdown complete");
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

    pub async fn announce(&mut self) -> Result<()> {
        let dht_record = match self.dht_record.take() {
            Some(dht_record) => dht_record,
            None => {
                self.routing_context
                    .create_dht_record(
                        veilid_core::DHTSchema::DFLT(DHTSchemaDFLT { o_cnt: 1 }),
                        None,
                    )
                    .await?
            }
        };
        let dht_key = dht_record.key().to_owned();
        self.dht_record = Some(dht_record);

        let (route_id, route_data) = self.routing_context.api().new_private_route().await?;
        self.route_id = Some(route_id);
        self.routing_context
            .set_dht_value(dht_key, 0, route_data)
            .await?;
        Ok(())
    }

    pub async fn serve(&self, remotes: Vec<String>) -> Result<()> {
        let token = CancellationToken::new();

        let callee = self.spawn_callee(token.clone());
        let mut futures = vec![callee];

        for remote in remotes.iter() {
            for _ in 1..10 {
                let caller = self.spawn_caller(token.clone(), remote.to_owned());
                futures.push(caller);
            }
        }

        let interrupter = tokio::spawn(async move {
            signal::ctrl_c().await?;
            warn!("interrupt received");
            token.cancel();
            Ok::<(), Error>(())
        });
        futures.push(interrupter);

        let _ = join_all(futures).await;
        Ok(())
    }

    fn spawn_caller(
        &self,
        token: CancellationToken,
        base_remote: String,
    ) -> JoinHandle<Result<()>> {
        let base_caller = self.clone();
        tokio::spawn(async move {
            loop {
                let caller = base_caller.clone();
                let remote = base_remote.clone();
                let remote_dht_key = TypedKey::from_str(&remote)?;
                let result: Result<()> = async {
                    let crypto = caller.routing_context.api().crypto()?.best();
                    let _ = caller
                        .routing_context
                        .open_dht_record(remote_dht_key.to_owned(), None)
                        .await;
                    let mut route_id: Option<RouteId> = None;
                    loop {
                        if token.is_cancelled() {
                            return Ok(());
                        }

                        let rid = match route_id {
                            Some(rid) => rid,
                            None => {
                                let route_data = caller
                                    .routing_context
                                    .get_dht_value(remote_dht_key.to_owned(), 0, true)
                                    .await?;
                                let route_blob = match route_data {
                                    Some(rd) => rd.data().to_vec(),
                                    None => {
                                        warn!("failed to get dht value");
                                        continue;
                                    }
                                };
                                caller
                                    .routing_context
                                    .api()
                                    .import_remote_private_route(route_blob)?
                            }
                        };
                        let msg = crypto.random_bytes(32768);
                        let msg_len = msg.len();
                        let resp = caller
                            .routing_context
                            .app_call(Target::PrivateRoute(rid), msg)
                            .await?;
                        debug!(sent = msg_len, received = resp.len());
                        route_id = Some(rid);
                    }
                }
                .await;
                if let Err(e) = result {
                    error!(err = format!("{}", e));
                }
                if token.is_cancelled() {
                    warn!("cancelled");
                    caller.routing_context.close_dht_record(remote_dht_key).await?;
                    return Ok(())
                }
            }
        })
    }

    fn spawn_callee(&self, token: CancellationToken) -> JoinHandle<Result<()>> {
        let mut callee = self.clone();
        tokio::spawn(async move {
            let mut bytesReceived = 0usize;
            let mut bytesSent = 0usize;
            let crypto = callee.routing_context.api().crypto()?.best();
            loop {
                select! {
                    res = callee.updates.recv_async() => {
                        let update = match res {
                            Ok(update) => update,
                            Err(e) => return Err(other_err(e)),
                        };
                        match update {
                            VeilidUpdate::AppCall(app_call) => {
                                debug!(received = app_call.message().len());
                                bytesReceived += app_call.message().len();
                                let resp = crypto.random_bytes(32768);

                                let rc = callee.routing_context.clone();
                                tokio::spawn(async move {
                                    rc.api().app_call_reply(app_call.id(), resp).await?;
                                    debug!(sent = app_call.message().len());
                                    Ok::<(), Error>(())
                                    // bytesSent ack on channel back to serve loop to update stats
                                });
                            }
                            VeilidUpdate::RouteChange(route_change) => {
                                if let Some(ref route_id) = callee.route_id {
                                    if route_change.dead_routes.contains(route_id) {
                                        info!("reannounce private route");
                                        callee.announce().await?;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ = token.cancelled() => {
                        return Ok(());
                    }
                }
            }
        })
    }
}
