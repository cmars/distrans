use std::{future::Future, path::Path};

use tokio::sync::broadcast::Receiver;
use veilid_core::{CryptoKey, CryptoTyped, OperationId, Target, VeilidUpdate};

use distrans_fileindex::Index;

use crate::{proto::Header, Result};

pub type ShareKey = CryptoTyped<CryptoKey>;

pub trait Peer: Clone + Send {
    fn subscribe_veilid_update(&self) -> Receiver<VeilidUpdate>;

    fn reset(&mut self) -> impl Future<Output = Result<()>> + Send;
    fn shutdown(self) -> impl Future<Output = Result<()>> + Send;

    fn announce(
        &mut self,
        index: &Index,
    ) -> impl std::future::Future<Output = Result<(ShareKey, Target, Header)>> + Send;
    fn reannounce_route(
        &mut self,
        key: &ShareKey,
        prior_route: Option<Target>,
        index: &Index,
        header: &Header,
    ) -> impl std::future::Future<Output = Result<(Target, Header)>> + Send;

    fn resolve(
        &mut self,
        key: &ShareKey,
        root: &Path,
    ) -> impl std::future::Future<Output = Result<(Target, Header, Index)>> + Send;
    fn reresolve_route(
        &mut self,
        key: &ShareKey,
        prior_route: Option<Target>,
    ) -> impl Future<Output = Result<(Target, Header)>> + Send;

    fn request_block(
        &mut self,
        target: Target,
        piece: usize,
        block: usize,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;
    fn reply_block_contents(
        &mut self,
        call_id: OperationId,
        contents: &[u8],
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}

mod veilid_peer;
pub use veilid_peer::VeilidPeer;

mod resilient_peer;
pub use resilient_peer::ResilientPeer;
