@0xc46f97c2b79df618;

###################################################################
# DHT record structures
###################################################################

struct Sha256 {
  p0 @0 :UInt64;
  p1 @1 :UInt64;
  p2 @2 :UInt64;
  p3 @3 :UInt64;
}

const pieceMaxLength: UInt32 = 1048576;

struct Payload {
  # Metadata about the entire payload to be delivered.

  digest @0 :Sha256;
  length @1 :UInt64;
}

struct Piece {
  # Metadata about a piece of the payload.
  # Pieces are 1MB in size max.

  digest @0 :Sha256;
  length @1 :UInt32 = .pieceMaxLength;
}

struct File {
  # Metadata about files which exist within the payload.

  contents @0 :Slice;  # Where the contents are located in the payload.
  path @1 :Text;       # A suggested name for the file.
}

struct Slice {
  # Describe a contiguous stream of bytes within a payload.
  # Pieces are consumed in consecutive order from the starting
  # piece until the length is reached.

  startingPiece @0 :UInt32;
  pieceOffset @1 :UInt32;
  length @2 :UInt64;
}

struct Index {
  pieces @0 :List(Piece);
  files @1 :List(File);
}

struct PayloadHeader {
  # Describe the payload and how to get it.
  # This header may be found at subkey 0 of the payload DHT key.

  payload @0 :Payload;            # Identify the payload offered.
  subkeys @1 :UInt16;             # Number of DHT subkeys following this one; concatenate and decode to get the Index.
  route @2 :Data;                 # Private route over which to make payload requests to this peer.

  discoveryKey @3 :Data;          # Peer discovery DHT key. If missing, peer discovery is not available.
  # discoverySignature @4 :Data;  # TODO: consider cross-signing the discoveryKey with payloadKey's owner
}

struct DiscoveryHeader {
  # Advertise other peers sharing the same payload.
  # This header may be found at subkey 0 of the discovery DHT key.

  payload @0 :Payload;          # Repeat the payload topic of discovery. All peers' payloadKeys should match this. 
  subkeys @1 :UInt16;           # Number of DHT subkeys following this one; each subkey is an encoded DiscoveryPeer.
  
  payloadKey @2 :Data;          # Payload metadata DHT key. If missing, payload cannot be fetched from this peer, it's rendezvous-only.
  # payloadSignature @3 :Data;  # TODO: consider cross-signing the payloadKey with discoveryKey's owner
}

struct AdvertisedPeer {
  # A discoverable peer participating in the payload swarm.
  # These are encoded in subkeys 1..n of the discovery DHT key.

  payloadKey @0 :Data;          # Peer's Payload DHT key, if the peer offers payload services.
  discoveryKey @1 :Data;        # Peer's Discovery DHT key, if the peer is known to have discovery services.
  status @2 :PeerAdvertStatus;  # Status of peer, an opinion of this advertising node.
}

struct PeerAdvertStatus {
  # Advertised status of a peer.
  #
  # Peer advertisements should be considered artistic works of fiction and
  # falsehood which are useful for discovery, but only a fool would take them as
  # unbiased fact.
  #
  # Think of an advertisement as a chemical signal deposited by an ant towards
  # discovery of food. The process isn't perfect and ants can get stuck in a
  # "death spiral". They usually avoid this because ants tend to cooperate. An
  # evolutionary battle-tested survival strategy.
  #
  # A "death spiral" where all the nodes get stuck on misinformation is a
  # Sybil-like Denial-of-Service attack.
  #
  # A node has to start from some basis of even partial truth to have some hope
  # of not falling into a death spiral -- the initial payload & discovery node
  # cannot be completely hostile. Hopefully you got the key from a somewhat
  # reliable source. Sources (such as search indexes) that give out bad keys are
  # unlikely to be used for long.
  #
  # From there, a savvy node should regard advertisements as initial vectors for
  # its own discovery and prioritization of peers, subject to revision based on
  # its own direct experience. IOW nodes, "do your own research" and take
  # nothing here as authoritative.
 
  updatedAt @0 :UInt64;                # When this status was last updated.
  payloadState @1 :PeerAdvertState;    # State of this peer's payload services.
  discoveryState @2 :PeerAdvertState;  # State of this peer's discovery services.
}

enum PeerAdvertState {
  # Advertised state of the peer, a subjective opinion of the discovery node.
  # Why even advertise peers in a not-ok state? Several reasons:
  #
  # Subkeys are like arrays, not maps. A peer can't be removed from the middle
  # without rewriting all the ones that follow. It's more efficient to mark them
  # and maybe periodically clean them up (if it's even worth doing that...).
  #
  # Discovery implementations might not implement all the checks necessary to
  # make all the determinations. Even if they do, they're not authoritative.

  unknown @0;      # Peer state is unknown to this advertiser.
  ok @1;           # Peer is advertised as good. Do your own research though.
  unavailable @2;  # Peer was unavailable when this advertiser tries to contact it.
  badContent @3;   # This peer responds with bad content.
}

###################################################################
# app_call protocol structures
###################################################################

struct AdvertRequest {
  # Request advertised keys from a peer.
  #
  # A peer may make this request asynchronously to another peer requesting
  # blocks, to add it to the discoverable swarm, by back-tracing the route_id
  # from other app_calls.
  #
  # Peers MAY implement a policy requiring advert responses to prevent
  # undiscoverable leechers, but this would be a peer implementation decision,
  # not a mandatory part of the protocol.
}

struct AdvertResponse {
  # Response containing a peer's keys to advertise.
  #
  # The owner signatures give the discovery node assurance the response isn't
  # forged, and can at least be announced in an "unknown" state.

  payloadKey @0 :Data;          # Payload DHT key
  payloadSignature @1 :Data;    # Signature of payload DHT key with owner key; proof of ownership

  discoveryKey @2 :Data;        # Discovery DHT key
  discoverySignature @3 :Data;  # Signature of discovery DHT key with owner key; proof of ownership
}

struct BlockRequest {
  # Request a block

  piece @0 :UInt32;
  block @1 :UInt8;
}

# "BlockResponse" is a bare-naked [u8] usually 32k in length, no
# serde. It contains the contents of the payload at the requested block.

struct VerifiedRequest {
  # Request a bitmap index of payload pieces available for fetching.

  # startingPiece @0 :UInt32 = 0;  # TODO: offset necessary for payloads > 277 TB
}

# "VerifiedResponse" is a bare-naked [u8] up to 32k in length, no serde. It
# contains a bitmap of verified pieces which the peer has, which is available
# for fetching, offset by the startingPiece.
