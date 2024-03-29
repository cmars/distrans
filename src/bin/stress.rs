use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use distrans_peer::{other_err, veilid_config, Error, Result};

use clap::{command, Parser};
use flume::{unbounded, Receiver, Sender};
use futures::future::join_all;
use metrics::{counter, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use tokio::{
    select, signal,
    task::JoinHandle,
    time::{timeout, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;
use veilid_core::{
    DHTRecordDescriptor, FromStr, KeyPair, RouteId, RoutingContext, Sequencing,
    Target, TypedKey, VeilidUpdate,
};

// This utility seeks to answer some basic questions worth answering before
// investing more heavily in this platform:
//
// - How much throughput can Veilid sustain over time?
// - Is this throughput scalable?
//
// To do this, a stress utility performs two roles, as a placeholder for a distrans peer:
//
// - Maintain a private route and publish it at a DHT, handling requests by responding with 32k random byte responses.
// - Send requests (32k random bytes) to one or more private routes found by DHT key as fast as possible.
//
// Requests may be app_call or app_message. Both have tradeoffs.
//
// Measure the throughput and errors, to answer questions:
//
// - Assuming no protocol overhead, what is the theoretical lower bound on
//   receiving a Linux ISO's worth of bytes?
// - Is this acceptable for some use cases?
// - Which is more effective in terms of theoretical upper-bound throughput? app_call or app_message?
//

#[derive(Parser, Debug)]
#[command(name = "stress")]
#[command(bin_name = "stress")]
struct Cli {
    #[arg(long, env)]
    pub state_dir: String,

    #[arg(long, env)]
    pub addr: SocketAddr,

    #[arg(long, env)]
    pub peers: Vec<String>,
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

    let _ = PrometheusBuilder::new()
        .with_http_listener(cli.addr)
        .install()
        .expect("install prometheus exporter");

    let mut app = App::new(PathBuf::from(cli.state_dir).as_path())
        .await
        .expect("new app");
    app.wait_for_network().await.expect("network");
    app.announce().await.expect("announce");
    let dht_key = (&app.dht_record.as_ref()).unwrap().key().to_owned();
    println!("{}", dht_key);

    app.serve(cli.peers.clone()).await.expect("serve");

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
            None => self.open_or_create_dht_record().await?,
        };
        let dht_key = dht_record.key().to_owned();
        self.dht_record = Some(dht_record);

        let (route_id, route_data) = self.routing_context.api().new_private_route().await?;
        self.route_id = Some(route_id);
        self.routing_context
            .set_dht_value(dht_key, 0, route_data, None)
            .await?;
        Ok(())
    }

    async fn open_or_create_dht_record(&mut self) -> Result<DHTRecordDescriptor> {
        let ts = self.routing_context.api().table_store()?;
        let db = ts.open("distrans_peer", 2).await?;
        let maybe_dht_key = db.load_json(0, &[]).await?;
        let maybe_dht_owner_keypair = db.load_json(1, &[]).await?;
        if let (Some(dht_key), Some(dht_owner_keypair)) = (maybe_dht_key, maybe_dht_owner_keypair) {
            return Ok(self
                .routing_context
                .open_dht_record(dht_key, dht_owner_keypair)
                .await?);
        }
        let dht_rec = self
            .routing_context
            .create_dht_record(veilid_core::DHTSchema::dflt(1)?, None)
            .await?;
        let dht_owner = KeyPair::new(
            dht_rec.owner().to_owned(),
            dht_rec
                .owner_secret()
                .ok_or(other_err("expected dht owner secret"))?
                .to_owned(),
        );
        db.store_json(0, &[], dht_rec.key()).await?;
        db.store_json(1, &[], &dht_owner).await?;
        Ok(dht_rec)
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

                        let t0 = Instant::now();
                        let resp = timeout(
                            Duration::from_secs(5),
                            caller
                                .routing_context
                                .app_call(Target::PrivateRoute(rid), msg),
                        )
                        .await
                        .map_err(other_err)??;
                        let delta = t0.elapsed();
                        debug!(sent = msg_len, received = resp.len());
                        counter!("bytes_sent").increment(msg_len as u64);
                        counter!("bytes_received").increment(resp.len() as u64);
                        histogram!("call_time").record(delta);
                        route_id = Some(rid);
                    }
                }
                .await;
                if let Err(e) = result {
                    error!(err = format!("{}", e));
                }
                if token.is_cancelled() {
                    warn!("cancelled");
                    caller
                        .routing_context
                        .close_dht_record(remote_dht_key)
                        .await?;
                    return Ok(());
                }
            }
        })
    }

    fn spawn_callee(&self, token: CancellationToken) -> JoinHandle<Result<()>> {
        let mut callee = self.clone();
        tokio::spawn(async move {
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
                                let t0 = Instant::now();
                                debug!(received = app_call.message().len(), from = format!("{:?}", app_call.sender()));
                                counter!("bytes_received").increment(app_call.message().len() as u64);

                                let resp = crypto.random_bytes(32768);
                                let rc = callee.routing_context.clone();
                                tokio::spawn(async move {
                                    let resp_len = resp.len();
                                    rc.api().app_call_reply(app_call.id(), resp).await?;
                                    let delta = t0.elapsed();

                                    debug!(sent = resp_len);
                                    counter!("bytes_sent").increment(resp_len as u64);
                                    histogram!("reply_time").record(delta);
                                    Ok::<(), Error>(())
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
