use std::{fmt::Display, path::Path, time::Duration};

use distrans_fileindex::Index;
use tokio::{
    select,
    sync::{broadcast::Receiver, watch},
    time::sleep,
};
use tracing::{debug, instrument, warn};
use veilid_core::{OperationId, Target, VeilidUpdate};

use crate::{error::Error, error::NodeState, error::Result, proto::Header, Peer};

use super::ShareKey;

pub struct Observable<P: Peer> {
    peer: P,
    node_state_rx: watch::Receiver<NodeState>,
    peer_progress_tx: watch::Sender<Progress>,
}

#[derive(Clone)]
pub struct Progress {
    pub state: State,
    pub length: u64,
    pub position: u64,
}

#[derive(Clone)]
pub enum State {
    Starting,
    Connecting,
    Announcing,
    Resolving,
    Connected,
    Down,
}

impl Default for Progress {
    fn default() -> Self {
        Progress {
            state: State::Starting,
            length: 0u64,
            position: 0u64,
        }
    }
}

impl<P: Peer + 'static> Observable<P> {
    const DEFAULT_RESET_TIMEOUT: Duration = Duration::from_secs(180);

    pub fn new(peer: P) -> Observable<P> {
        let updates = peer.subscribe_veilid_update();
        let (tx, rx) = watch::channel(NodeState::APINotStarted);
        let (peer_progress_tx, _) = watch::channel(Progress::default());
        tokio::spawn(Self::track_node_state(updates, tx));
        Observable {
            peer,
            node_state_rx: rx,
            peer_progress_tx,
        }
    }

    async fn track_node_state(
        mut updates: Receiver<VeilidUpdate>,
        tx: tokio::sync::watch::Sender<NodeState>,
    ) -> Result<()> {
        loop {
            let update = updates.recv().await.map_err(Error::other)?;
            match update {
                VeilidUpdate::Attachment(attachment) => {
                    let is_attach = match attachment.state {
                        veilid_core::AttachmentState::Detached => false,
                        veilid_core::AttachmentState::Attaching => true,
                        veilid_core::AttachmentState::AttachedWeak => true,
                        veilid_core::AttachmentState::AttachedGood => true,
                        veilid_core::AttachmentState::AttachedStrong => true,
                        veilid_core::AttachmentState::FullyAttached => true,
                        veilid_core::AttachmentState::OverAttached => true,
                        _ => false,
                    };
                    let updated_state = if attachment.public_internet_ready {
                        NodeState::Connected
                    } else if is_attach {
                        NodeState::Connecting
                    } else {
                        NodeState::NetworkNotAvailable
                    };
                    debug!(
                        state = format!("{}", updated_state),
                        attachment = format!("{:?}", attachment)
                    );
                    tx.send(updated_state).map_err(Error::other)?;
                }
                VeilidUpdate::Shutdown => {
                    debug!(state = format!("{}", NodeState::APIShuttingDown));
                    tx.send(NodeState::APIShuttingDown).map_err(Error::other)?;
                    break;
                }
                _ => {}
            }
        }
        Ok::<(), Error>(())
    }

    pub fn subscribe_peer_progress(&self) -> watch::Receiver<Progress> {
        self.peer_progress_tx.subscribe()
    }

    fn update_progress(peer_progress_tx: &watch::Sender<Progress>, state: State) {
        warn_err(peer_progress_tx.send(Progress {
            state,
            length: 0u64,
            position: 0u64,
        }));
    }
}

impl<P: Peer + 'static> Peer for Observable<P> {
    fn subscribe_veilid_update(&self) -> Receiver<VeilidUpdate> {
        self.peer.subscribe_veilid_update()
    }

    #[instrument(skip(self), level = "debug", err)]
    async fn reset(&mut self) -> Result<()> {
        Self::update_progress(&self.peer_progress_tx, State::Connecting);
        self.peer.reset().await?;
        select! {
            wait_result = self.node_state_rx.wait_for(NodeState::is_connected) => {
                if let Err(e) = wait_result {
                    return Err(Error::other(e));
                }
                Self::update_progress(&self.peer_progress_tx, State::Connected);
                return Ok(());
            }
            _ = sleep(Self::DEFAULT_RESET_TIMEOUT) => {
                Self::update_progress(&self.peer_progress_tx, State::Down);
                return Err(Error::ResetTimeout);
            }
        }
    }

    #[instrument(skip(self), level = "debug", err)]
    async fn shutdown(self) -> Result<()> {
        Self::update_progress(&self.peer_progress_tx, State::Down);
        self.peer.shutdown().await
    }

    #[instrument(skip(self, index), level = "debug", err)]
    async fn announce(&mut self, index: &Index) -> Result<(ShareKey, Target, Header)> {
        self.peer.announce(index).await
    }

    #[instrument(skip(self, index, header), level = "debug", err)]
    async fn reannounce_route(
        &mut self,
        key: &ShareKey,
        prior_route: Option<Target>,
        index: &Index,
        header: &Header,
    ) -> Result<(Target, Header)> {
        self.peer
            .reannounce_route(key, prior_route, index, header)
            .await
    }

    #[instrument(skip(self), level = "debug", err)]
    async fn resolve(&mut self, key: &ShareKey, root: &Path) -> Result<(Target, Header, Index)> {
        self.peer.resolve(key, root).await
    }

    #[instrument(skip(self), level = "debug", err)]
    async fn reresolve_route(
        &mut self,
        key: &ShareKey,
        prior_route: Option<Target>,
    ) -> Result<(Target, Header)> {
        self.peer.reresolve_route(key, prior_route).await
    }

    #[instrument(skip(self), level = "trace", err)]
    async fn request_block(
        &mut self,
        target: Target,
        piece: usize,
        block: usize,
    ) -> Result<Vec<u8>> {
        self.peer.request_block(target, piece, block).await
    }

    #[instrument(skip(self, contents), level = "trace", err)]
    async fn reply_block_contents(&mut self, call_id: OperationId, contents: &[u8]) -> Result<()> {
        self.peer.reply_block_contents(call_id, contents).await
    }
}

fn warn_err<T, E: Display>(result: std::result::Result<T, E>) {
    if let Err(e) = result {
        warn!(err = format!("{}", e));
    }
}

impl<P: Peer> Clone for Observable<P> {
    fn clone(&self) -> Self {
        Observable {
            peer: self.peer.clone(),
            node_state_rx: self.node_state_rx.clone(),
            peer_progress_tx: self.peer_progress_tx.clone(),
        }
    }
}
