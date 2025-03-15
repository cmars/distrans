use std::path::PathBuf;

use sha2::{Digest as _, Sha256};
use stigmerge_fileindex::Index;
use tokio::select;
use tokio_util::sync::CancellationToken;
use veilid_core::Target;

use crate::{
    peer::TypedKey,
    proto::{Digest, Encoder, Header},
    Error, Peer, Result,
};

use super::ChanServer;

pub(super) enum Request {
    Index {
        key: TypedKey,
        want_index_digest: Digest,
        root: PathBuf,
    },
    Header {
        key: TypedKey,
        prior_target: Option<Target>,
    },
    Remove {
        key: TypedKey,
    },
}

impl Request {
    fn key(&self) -> &TypedKey {
        match self {
            Request::Index {
                key,
                want_index_digest: _,
                root: _,
            } => key,
            Request::Header {
                key,
                prior_target: _,
            } => key,
            Request::Remove { key } => key,
        }
    }
}

pub(super) enum Response {
    NotAvailable {
        key: TypedKey,
        err: Error,
    },
    BadIndex {
        key: TypedKey,
    },
    Index {
        key: TypedKey,
        header: Header,
        index: Index,
        target: Target,
    },
    Header {
        key: TypedKey,
        header: Header,
        target: Target,
    },
}

pub(super) struct Service<P: Peer> {
    peer: P,
    ch: ChanServer<Request, Response>,
}

impl<P: Peer> Service<P> {
    pub(super) fn new(peer: P, ch: ChanServer<Request, Response>) -> Self {
        Self { peer, ch }
    }

    pub async fn run(mut self, cancel: CancellationToken) -> Result<()> {
        loop {
            select! {
                _ = cancel.cancelled() => {
                    return Ok(())
                }
                res = self.ch.rx.recv() => {
                    let req = match res {
                        None => return Ok(()),
                        Some(req) => req,
                    };
                    match self.resolve(&req).await {
                        // TODO: set up share header watches
                        Ok(resp) => self.ch.tx.send(resp).await.map_err(Error::other)?,
                        Err(err) => self.ch.tx.send(Response::NotAvailable{key: req.key().clone(), err}).await.map_err(Error::other)?,
                    }
                }
            }
        }
    }

    async fn resolve(&mut self, req: &Request) -> Result<Response> {
        Ok(match req {
            Request::Index {
                key,
                want_index_digest,
                root,
            } => {
                let (target, header, index) = self.peer.resolve(key, root.as_path()).await?;
                let mut peer_index_digest = Sha256::new();
                peer_index_digest.update(index.encode().map_err(Error::other)?);
                if peer_index_digest.finalize().as_slice() == want_index_digest {
                    Response::Index {
                        key: key.clone(),
                        header,
                        index,
                        target,
                    }
                } else {
                    Response::BadIndex { key: key.clone() }
                }
            }
            Request::Header {
                ref key,
                prior_target,
            } => {
                let (target, header) = self.peer.reresolve_route(key, *prior_target).await?;
                Response::Header {
                    key: key.clone(),
                    header,
                    target,
                }
            }
            // TODO: remove watch on this key if one exists
            Request::Remove { key: _ } => todo!(),
        })
    }
}
