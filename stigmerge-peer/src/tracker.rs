use std::{collections::HashMap, path::Path};

use roaring::RoaringBitmap;
use sha2::{Digest as _, Sha256};
use stigmerge_fileindex::Index;
use tokio::{select, time::Instant};
use tokio_util::sync::CancellationToken;
use veilid_core::{DHTRecordDescriptor, Target};

use crate::{
    peer::ShareKey,
    proto::{encode_index, Digest, Header},
    Error, Peer, Result,
};

pub struct Syncer<P: Peer> {
    peer: P,
    have_index: Index,
    want_index_digest: Digest,
    want_index: Option<Index>,
    members: Vec<ShareKey>,

    seeker: Option<Seeker>,
    sharer: Option<Sharer>,

    // Are there held by the sharer?
    share_key: Option<DHTRecordDescriptor>,
    peermap_key: Option<DHTRecordDescriptor>,

    // Is this held by the seeker?
    havemap_key: Option<DHTRecordDescriptor>,
}

impl<P: Peer> Syncer<P> {
    /// Create a new Syncer with the digest of the desired content state, a
    /// local index representing local filesystem state (which could be anything
    /// from empty to complete), and some peer DHT keys to synchronize with.
    pub fn new(
        peer: P,
        want_index_digest: Digest,
        have_index: Index,
        members: &[ShareKey],
    ) -> Result<Self> {
        // Do we already have what we want?
        let mut have_index_digest = Sha256::new();
        have_index_digest.update(encode_index(&have_index).map_err(Error::other)?.as_slice());
        let have_index_digest_bytes: Digest = have_index_digest.finalize().into();

        Ok(Self {
            peer,
            want_index_digest,
            want_index: if have_index_digest_bytes == want_index_digest {
                Some(have_index.clone())
            } else {
                None
            },
            have_index,
            members: members.to_vec(),

            seeker: None,
            sharer: None,
            havemap_key: None,
            peermap_key: None,
            share_key: None,
        })
    }

    pub async fn swarm(&mut self, cancel: CancellationToken) -> Result<()> {
        loop {
            select! {
                _ = cancel.cancelled() => {
                    return Ok(())
                }
                // react to sharer events?
                // react to seeker events?
            }
        }
    }

    /// The DHT share key for this peer, with which other peers can swarm on the
    /// content.
    pub fn share_key(&self) -> Option<ShareKey> {
        todo!()
    }
}

struct RemotePeerInfo {
    share_key: ShareKey,

    havemap_key: Option<ShareKey>,
    peermap_key: Option<ShareKey>,

    /// Last known private route and share DHT header for this peer.
    route: Option<(Target, Header)>,

    /// Last known advertisted
    /// May be updated by watches on an active peer.
    havemap: Option<RoaringBitmap>,

    /// Last known advertised peermap.
    /// May be updated by watches on an active peer.
    peermap: Vec<AdvertisedPeer>,

    /// When the Peer's havemap was last synchronized.
    havemap_updated_at: Option<Timestamp>,

    /// When the Peer's peermap was last synchronized.
    peermap_updated_at: Option<Timestamp>,

    /// When the Peer last provided a block response over the private route.
    last_block_response: Option<Timestamp>,

    /// This peer advertises a havemap DHT key that isn't available.
    missing_havemap: bool,

    /// This peer advertises a peermap DHT key that isn't available.
    missing_peermap: bool,

    /// This peer's share DHT key isn't available.
    missing_share: bool,

    /// This peer has a share DHT, but the values unmarshal to an invalid index.
    bad_index: bool,

    /// This peer provided bad blocks.
    bad_blocks: bool,
}

impl RemotePeerInfo {
    fn new(share_key: ShareKey) -> Self {
        Self {
            share_key,
            havemap_key: None,
            peermap_key: None,
            route: None,
            havemap: None,
            peermap: vec![],
            havemap_updated_at: None,
            peermap_updated_at: None,
            last_block_response: None,
            missing_havemap: false,
            missing_peermap: false,
            missing_share: false,
            bad_index: false,
            bad_blocks: false,
        }
    }

    fn need_havemap_dht(&self) -> bool {
        if let None = self.havemap_key {
            true
        } else {
            false
        }
    }

    fn need_havemap(&self) -> bool {
        if let None = self.havemap {
            true
        } else {
            false
        }
    }
}

struct AdvertisedPeer {
    share_key: ShareKey,
    updated_at: Timestamp,
}

struct Tracker {
    /// Digest of the wanted index.
    want_index_digest: Digest,

    /// The wanted index, once resolved with a peer.
    /// It's authenticated by matching the index digest.
    want_index: Option<Index>,

    /// Collection of known remote peers.
    peers: HashMap<ShareKey, RemotePeerInfo>,
}

impl Tracker {
    fn new(want_index_digest: Digest, share_keys: Vec<ShareKey>) -> Tracker {
        Tracker {
            want_index_digest,
            want_index: None,
            peers: share_keys
                .map(|k| (k.clone(), RemotePeerInfo::new(k)))
                .collect(),
        }
    }
}

/// Sharer advertises peers and serves blocks to others.
struct Sharer {}
