use std::{path::Path, time::Duration};

use backoff::{backoff::Backoff, ExponentialBackoff};
use distrans_fileindex::Index;
use tokio::{
    select,
    sync::{broadcast::Receiver, watch},
    time::sleep,
};
use tracing::{debug, instrument};
use veilid_core::{OperationId, Target, VeilidAPIError, VeilidUpdate};

use crate::{
    error::Error,
    error::Result,
    error::{NodeState, Unexpected},
    proto::Header,
    Peer,
};

use super::ShareKey;

pub struct ResilientPeer<P: Peer> {
    peer: P,
    node_state_rx: tokio::sync::watch::Receiver<NodeState>,
    initial_retry_backoff: ExponentialBackoff,
    initial_reset_backoff: ExponentialBackoff,
}

impl<P: Peer + 'static> ResilientPeer<P> {
    pub fn new(p: P) -> ResilientPeer<P> {
        let updates = p.subscribe_veilid_update();
        let (tx, rx) = watch::channel(NodeState::APINotStarted);
        tokio::spawn(Self::track_node_state(updates, tx));
        ResilientPeer {
            peer: p,
            node_state_rx: rx,
            initial_retry_backoff: Self::default_retry_backoff(),
            initial_reset_backoff: Self::default_reset_backoff(),
        }
    }

    pub fn with_backoff(
        self,
        retry_backoff: ExponentialBackoff,
        reset_backoff: ExponentialBackoff,
    ) -> Self {
        ResilientPeer {
            peer: self.peer,
            node_state_rx: self.node_state_rx,
            initial_retry_backoff: retry_backoff,
            initial_reset_backoff: reset_backoff,
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

    fn is_retryable(err: &Error) -> bool {
        match err {
            Error::Fault(_) => true,
            Error::Index { path: _, err: _ } => false,
            Error::LocalFile(_) => false,
            Error::Node { state, err: _ } => *state == NodeState::RemotePeerNotAvailable,
            Error::RemoteProtocol(_) => true,
            Error::InternalProtocol(_) => false,
        }
    }

    fn is_resettable(err: &Error) -> bool {
        match err {
            Error::Index { path: _, err: _ } => false,
            Error::LocalFile(_) => false,
            Error::InternalProtocol(_) => false,
            Error::Node { state, err: _ } => match state {
                NodeState::APIShuttingDown => false,
                NodeState::PrivateRouteInvalid => false,
                _ => true,
            },
            Error::Fault(err) => match err {
                Unexpected::Veilid(VeilidAPIError::Unimplemented { message: _ }) => false,
                Unexpected::Veilid(_) => true,
                _ => false,
            },
            _ => true,
        }
    }

    fn default_retry_backoff() -> ExponentialBackoff {
        let mut back_off = ExponentialBackoff::default();
        back_off.max_elapsed_time = Some(Duration::from_secs(120));
        back_off
    }

    fn default_reset_backoff() -> ExponentialBackoff {
        let mut back_off = ExponentialBackoff::default();
        back_off.max_elapsed_time = None;
        back_off
    }

    fn retry_backoff(&self) -> ExponentialBackoff {
        self.initial_retry_backoff.clone()
    }

    fn reset_backoff(&self) -> ExponentialBackoff {
        self.initial_reset_backoff.clone()
    }

    async fn retry_delay(&mut self, back_off: &mut ExponentialBackoff, err: Error) -> Result<()> {
        if Self::is_retryable(&err) {
            if let Some(delay) = back_off.next_backoff() {
                debug!(
                    err = format!("{}", err),
                    delay = format!("{:?}", delay),
                    "retry"
                );
                tokio::time::sleep(delay).await;
                return Ok(());
            }
        }
        if Self::is_resettable(&err) {
            debug!(err = format!("{}", err), "resetting connection");
            self.reset().await?;
            back_off.reset();
            return Ok(());
        }
        Err(err)
    }
}

impl<P: Peer + 'static> Peer for ResilientPeer<P> {
    fn subscribe_veilid_update(&self) -> Receiver<VeilidUpdate> {
        self.peer.subscribe_veilid_update()
    }

    #[instrument(skip(self), level = "debug", err)]
    async fn reset(&mut self) -> Result<()> {
        let mut back_off = self.reset_backoff();
        loop {
            match self.peer.reset().await {
                Ok(_) => {
                    select! {
                        wait_result = self.node_state_rx.wait_for(NodeState::is_connected) => {
                            if let Err(e) = wait_result {
                                return Err(Error::other(e));
                            }
                            return Ok(());
                        }
                        _ = sleep(Duration::from_secs(120)) => {
                            continue;
                        }
                    }
                }
                Err(e) => {
                    if Self::is_retryable(&e) || Self::is_resettable(&e) {
                        if let Some(delay) = back_off.next_backoff() {
                            select! {
                                _ = sleep(delay) => continue,
                                watch_result = self.node_state_rx.wait_for(NodeState::is_connected) => {
                                    if let Err(_) = watch_result {
                                        return Err(e);
                                    }
                                    return Ok(());
                                }
                            }
                        }
                    }
                    return Err(e);
                }
            }
        }
    }

    #[instrument(skip(self), level = "debug", err)]
    async fn shutdown(self) -> Result<()> {
        self.peer.shutdown().await
    }

    #[instrument(skip(self, index), level = "debug", err)]
    async fn announce(&mut self, index: &Index) -> Result<(ShareKey, Target, Header)> {
        let mut back_off = self.retry_backoff();
        loop {
            match self.peer.announce(index).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    self.retry_delay(&mut back_off, e).await?;
                    continue;
                }
            }
        }
    }

    #[instrument(skip(self, index, header), level = "debug", err)]
    async fn reannounce_route(
        &mut self,
        key: &ShareKey,
        prior_route: Option<Target>,
        index: &Index,
        header: &Header,
    ) -> Result<(Target, Header)> {
        let mut back_off = self.retry_backoff();
        loop {
            match self
                .peer
                .reannounce_route(key, prior_route, index, header)
                .await
            {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    self.retry_delay(&mut back_off, e).await?;
                    continue;
                }
            }
        }
    }

    #[instrument(skip(self), level = "debug", err)]
    async fn resolve(&mut self, key: &ShareKey, root: &Path) -> Result<(Target, Header, Index)> {
        let mut back_off = self.retry_backoff();
        loop {
            match self.peer.resolve(key, root).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    self.retry_delay(&mut back_off, e).await?;
                    continue;
                }
            }
        }
    }

    #[instrument(skip(self), level = "debug", err)]
    async fn reresolve_route(
        &mut self,
        key: &ShareKey,
        prior_route: Option<Target>,
    ) -> Result<(Target, Header)> {
        let mut back_off = self.retry_backoff();
        loop {
            match self.peer.reresolve_route(key, prior_route).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    self.retry_delay(&mut back_off, e).await?;
                    continue;
                }
            }
        }
    }

    #[instrument(skip(self), level = "debug", err)]
    async fn request_block(
        &mut self,
        target: Target,
        piece: usize,
        block: usize,
    ) -> Result<Vec<u8>> {
        let mut back_off = self.retry_backoff();
        loop {
            match self.peer.request_block(target, piece, block).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    self.retry_delay(&mut back_off, e).await?;
                    continue;
                }
            }
        }
    }

    #[instrument(skip(self, contents), level = "debug", err)]
    async fn reply_block_contents(&mut self, call_id: OperationId, contents: &[u8]) -> Result<()> {
        let mut back_off = ExponentialBackoff::default();
        loop {
            match self.peer.reply_block_contents(call_id, contents).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    self.retry_delay(&mut back_off, e).await?;
                    continue;
                }
            }
        }
    }
}

impl<P: Peer> Clone for ResilientPeer<P> {
    fn clone(&self) -> Self {
        ResilientPeer {
            peer: self.peer.clone(),
            node_state_rx: self.node_state_rx.clone(),
            initial_retry_backoff: self.initial_retry_backoff.clone(),
            initial_reset_backoff: self.initial_reset_backoff.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use veilid_core::{FromStr, TypedKey, VeilidStateAttachment};

    use crate::proto;
    use crate::tests::StubPeer;

    use super::*;

    #[tokio::test]
    async fn reset_ok() {
        let mut stub_peer = StubPeer::new();
        let update_tx = stub_peer.update_tx.clone();
        let reset_calls = Arc::new(Mutex::new(0));
        let reset_calls_internal = reset_calls.clone();
        stub_peer.reset_result = Arc::new(Mutex::new(move || {
            (*(reset_calls_internal.lock().unwrap())) += 1;
            Ok(())
        }));
        let mut rp = ResilientPeer::new(stub_peer);
        // Simulate getting connected to network, normally track_node_state
        // would set this when the node comes online.
        update_tx
            .send(VeilidUpdate::Attachment(Box::new(VeilidStateAttachment {
                state: veilid_core::AttachmentState::AttachedGood,
                public_internet_ready: true,
                local_network_ready: true,
            })))
            .expect("send veilid update");
        rp.reset().await.unwrap();
        // Simulate a shutdown so that track_node_state exits.
        update_tx
            .send(VeilidUpdate::Shutdown)
            .expect("send veilid update");
        // reset was called once
        assert_eq!(*(reset_calls.lock().unwrap()), 1);
    }

    #[tokio::test]
    async fn reset_recover() {
        let mut stub_peer = StubPeer::new();
        let update_tx = stub_peer.update_tx.clone();
        let update_tx_internal = update_tx.clone();
        let reset_calls = Arc::new(Mutex::new(0));
        let reset_calls_internal = reset_calls.clone();
        stub_peer.reset_result = Arc::new(Mutex::new(move || {
            let mut calls = reset_calls_internal.lock().unwrap();
            let result = if *calls < 5 {
                // Send a retryable error that causes reset to backoff-retry
                Err(Error::RemoteProtocol(proto::Error::Other(
                    "bad response".to_string(),
                )))
            } else {
                // Simulate "coming online"
                update_tx_internal
                    .send(VeilidUpdate::Attachment(Box::new(VeilidStateAttachment {
                        state: veilid_core::AttachmentState::AttachedGood,
                        public_internet_ready: true,
                        local_network_ready: true,
                    })))
                    .expect("send veilid update");
                Ok(())
            };
            *calls += 1;
            result
        }));
        let mut rp = ResilientPeer::new(stub_peer);
        // Zero out the backoff retry delays for a faster test
        rp.initial_reset_backoff.initial_interval = Duration::ZERO;
        rp.initial_reset_backoff.multiplier = 0.0;
        // reset eventually succeeds
        rp.reset().await.unwrap();
        // on the 6th attempt
        assert_eq!(*(reset_calls.lock().unwrap()), 6);
        update_tx
            .send(VeilidUpdate::Shutdown)
            .expect("send veilid update");
    }

    #[tokio::test]
    async fn reset_nonrecoverable() {
        let mut stub_peer = StubPeer::new();
        let update_tx = stub_peer.update_tx.clone();
        let reset_calls = Arc::new(Mutex::new(0));
        let reset_calls_internal = reset_calls.clone();
        stub_peer.reset_result = Arc::new(Mutex::new(move || {
            let mut calls = reset_calls_internal.lock().unwrap();
            *calls += 1;
            // Return an error that is nonrecoverable, even for a reset.
            // An internal protocol error isn't something we'd normally send in a reset
            // but it's indicative of an internal failure to serialize a message, which
            // isn't going to get better if we have such a bug.
            Err(Error::InternalProtocol(proto::Error::Other(
                "nope".to_string(),
            )))
        }));
        let mut rp = ResilientPeer::new(stub_peer);
        let result = rp.reset().await;
        // reset failed with the nonrecoverable error
        assert!(matches!(result, Err(Error::InternalProtocol(_))));
        update_tx
            .send(VeilidUpdate::Shutdown)
            .expect("send veilid update");
        // reset was called once
        assert_eq!(*(reset_calls.lock().unwrap()), 1);
    }

    #[tokio::test]
    async fn reset_retry_exceeded() {
        let mut stub_peer = StubPeer::new();
        let update_tx = stub_peer.update_tx.clone();
        let reset_calls = Arc::new(Mutex::new(0));
        let reset_calls_internal = reset_calls.clone();
        stub_peer.reset_result = Arc::new(Mutex::new(move || {
            let mut calls = reset_calls_internal.lock().unwrap();
            *calls += 1;
            // Return a recoverable error
            Err(Error::Fault(Unexpected::Veilid(
                VeilidAPIError::Unimplemented {
                    message: "nope".to_string(),
                },
            )))
        }));
        let mut rp = ResilientPeer::new(stub_peer);
        // Configure the retry threshold to zero, simulating a backoff retries
        // exceeded condition.
        rp.initial_reset_backoff.max_elapsed_time = Some(Duration::ZERO);
        let result = rp.reset().await;
        // Last error returned
        assert!(matches!(result, Err(Error::Fault(Unexpected::Veilid(_)))));
        update_tx
            .send(VeilidUpdate::Shutdown)
            .expect("send veilid update");
        // reset calledat least once
        assert!(*(reset_calls.lock().unwrap()) >= 1);
    }

    #[tokio::test]
    async fn request_block_ok() {
        let mut stub_peer = StubPeer::new();
        let update_tx = stub_peer.update_tx.clone();
        let calls = Arc::new(Mutex::new(0));
        let calls_internal = calls.clone();
        stub_peer.request_block_result = Arc::new(Mutex::new(move || {
            // Simulate a request_block peer call that succeeds.
            (*(calls_internal.lock().unwrap())) += 1;
            Ok(vec![0xde, 0xad, 0xbe, 0xef])
        }));
        let mut rp = ResilientPeer::new(stub_peer);
        update_tx
            .send(VeilidUpdate::Attachment(Box::new(VeilidStateAttachment {
                state: veilid_core::AttachmentState::AttachedGood,
                public_internet_ready: true,
                local_network_ready: true,
            })))
            .expect("send veilid update");
        // Fake a target
        let key = TypedKey::from_str("VLD0:cCHB85pEaV4bvRfywxnd2fRNBScR64UaJC8hoKzyr3M")
            .expect("parsed key");
        let response = rp
            .request_block(Target::PrivateRoute(key.value), 0, 0)
            .await
            .unwrap();
        // Got simulated block contents
        assert_eq!(response, vec![0xde, 0xad, 0xbe, 0xef]);
        update_tx
            .send(VeilidUpdate::Shutdown)
            .expect("send veilid update");
        // request_block called once
        assert_eq!(*(calls.lock().unwrap()), 1);
    }

    #[tokio::test]
    async fn request_block_recover() {
        let mut stub_peer = StubPeer::new();
        let update_tx = stub_peer.update_tx.clone();
        let calls = Arc::new(Mutex::new(0));
        let calls_internal = calls.clone();
        stub_peer.request_block_result = Arc::new(Mutex::new(move || {
            let mut call = calls_internal.lock().unwrap();
            let result = if *call < 5 {
                // Temporary recoverable failure condition.
                // The remote peer is responding with garbage.
                Err(Error::RemoteProtocol(proto::Error::Other(
                    "nope".to_string(),
                )))
            } else {
                // Finally got a good response.
                Ok(vec![0xde, 0xad, 0xbe, 0xef])
            };
            *call += 1;
            result
        }));
        let mut rp = ResilientPeer::new(stub_peer);
        // Zero out backoff delays for speed.
        rp.initial_retry_backoff.initial_interval = Duration::ZERO;
        rp.initial_retry_backoff.multiplier = 0.0;
        update_tx
            .send(VeilidUpdate::Attachment(Box::new(VeilidStateAttachment {
                state: veilid_core::AttachmentState::AttachedGood,
                public_internet_ready: true,
                local_network_ready: true,
            })))
            .expect("send veilid update");
        let key = TypedKey::from_str("VLD0:cCHB85pEaV4bvRfywxnd2fRNBScR64UaJC8hoKzyr3M")
            .expect("parsed key");
        let response = rp
            .request_block(Target::PrivateRoute(key.value), 0, 0)
            .await
            .unwrap();
        // Got simulated block contents
        assert_eq!(response, vec![0xde, 0xad, 0xbe, 0xef]);
        update_tx
            .send(VeilidUpdate::Shutdown)
            .expect("send veilid update");
        // 6th time's a charm
        assert_eq!(*(calls.lock().unwrap()), 6);
    }

    #[tokio::test]
    async fn request_block_nonrecoverable() {
        let mut stub_peer = StubPeer::new();
        let update_tx = stub_peer.update_tx.clone();
        stub_peer.request_block_result = Arc::new(Mutex::new(move || {
            // It's not you it's me
            Err(Error::InternalProtocol(proto::Error::Other(
                "nope".to_string(),
            )))
        }));
        let mut rp = ResilientPeer::new(stub_peer);
        update_tx
            .send(VeilidUpdate::Attachment(Box::new(VeilidStateAttachment {
                state: veilid_core::AttachmentState::AttachedGood,
                public_internet_ready: true,
                local_network_ready: true,
            })))
            .expect("send veilid update");
        let key = TypedKey::from_str("VLD0:cCHB85pEaV4bvRfywxnd2fRNBScR64UaJC8hoKzyr3M")
            .expect("parsed key");
        let result = rp
            .request_block(Target::PrivateRoute(key.value), 0, 0)
            .await;
        assert!(matches!(
            result,
            Err(Error::InternalProtocol(proto::Error::Other(_)))
        ));
        update_tx
            .send(VeilidUpdate::Shutdown)
            .expect("send veilid update");
    }
}
