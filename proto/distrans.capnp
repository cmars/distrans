@0xc46f97c2b79df618;

# # Distrans protocol definitions
#
# Distrans protocol design needs to bridge two abstractions:
#
# 1. How to effectively distribute content among blocks which can be concurrently send and received
# 2. How to map the content in those blocks onto the local filesystem
#
# (1) is constrained by Veilid's DHT and app_call limits (32k max length for a message or DHT value).
# (2) is ideal for file-sharing because we don't want to duplicate storage
#     locally: serve the files which are assembled.
#

struct Layout {
  name @0 :Text;
  files @1 :List(FileSpec);
}

struct FileSpec {
  length @0 :UInt64;
  digest @1 :SHA256;
  path @2 :List(Text);
}

struct Index {
  pieceLength @0 :UInt64;
  pieces @1 :List(SHA256);
}

# # Veilid DHT organization
#
# +------------------------+
# | Layout                 | Subkey 0
# +------------------------+
# | Index                  | Subkey 1
# +------------------------+
# | TrackerInfo            | Subkey 2
# +------------------------+
# | PeerStatus             | Subkeys 3..SUBKEY_MAX
# +------------------------+
#
# Transport.pieceLength * Transport.pieces.length >= sum(Layout.files[].length)
#
# where the last piece may be < Transport.pieceLength.

struct TrackerInfo {
  route @0 :Data;
  # Private route to contact this tracker via app_call.

  digest @1 :SHA256;
  # Digest of the encoded Transport struct; this uniquely identifies the content
  # being delivered by this tracker.
}

struct PeerStatus {
  route @0 :Data;
  lastUpdatedAt @1 :UInt64;

  claims @2 :Data;
  # Bitmap of complete pieces or blocks. Do not lie, you may be tested.
}

struct Request {
  # Protocol app_call request messages

  union {
    peerJoin @0 :PeerJoinParams;
    # Peer-to-Tracker request to join the swarm

    peerStatus @1 :Void;
    # Any-to-Peer, request for status

    pieceBlock @2 :PieceBlockParams;
    # Peer-to-Peer, request for a piece block
  }
}

struct PeerJoinParams {
  # A peer joins the horde by making an app_call request to the tracker, who sets
  # its info in a free DHT subkey.

  digest @0 :SHA256;
  route @1 :Data;
  claims @2 :Data;
}

struct PieceBlockParams {
  pieceIndex @0 :UInt64;
  # Piece index, as mapped in Transport above

  blockIndex @1 :UInt8;
  # Block index within the part
  # Using a single byte for this caps the upper bound on piece length to 8Mb (minus protocol overhead)
}

struct Response {
  # Protocol app_call response messages

  union {
    peerJoin @0 :Void;

    peerStatus @1 :PeerStatus;

    pieceBlock @2 :PieceBlock;
  }
}

struct PieceBlock {
  pieceIndex @0 :UInt64;
  blockIndex @1 :UInt8;

  contents @2 :Data;
  # Contents of the block, which is limited by the Veilid transport.
  # Limited to 32256.
}

struct SHA256 {
  p0 @0 :UInt64;
  p1 @1 :UInt64;
  p2 @2 :UInt64;
  p3 @3 :UInt64;
}
