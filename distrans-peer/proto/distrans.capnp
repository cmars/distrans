@0xc46f97c2b79df618;

# Protocols within protocols within protocols

# Veilid DHT has some pretty severe limitations:
# - Max subkey value length 32KB
# - Max total DHT value length (sum of all subkey lengths) 1MB
#
# Some of the structures that need to be represented in order to
# index a large file could easily exceed these:
# - List of blocks and their content hashes
# - List of files and how they map onto block number and offset 
# - List of peers and their private routes

# The ground floor
#
# A Metadata Node serves as an envelope that can span DHT subkeys and keys.
# It's a simple linked list that points to the next subkey to append to the cumulative value.

struct MDNode {
  value @0 :Data;

  union {
    next @1 :MDNodeRef;
    nil @2 :Void;
  }
}

using Subkey = UInt32;
using TypedKey = Data;

struct MDNodeRef {
  # An MDNodeRef can either be local to the same DHT record, or remote (another DHT key).
 
  union {
    local @0 :Subkey;
    remote @1 :RemoteMDNodeRef;
  }
}

struct RemoteMDNodeRef {
  # A RemoteNodeRef allows content to spill out into another DHT record entirely.
  # This may be necessary in order to exceed the 1MB total size limit per DHT key.

  key @0 :TypedKey;
  subkey @1 :Subkey;
}

# Blocks are 1MB in size

# Files are overlaid on top of 1MB-sized blocks

struct FileIndex {
  block @0 :UInt32;   # 4 Exabytes is probably enough (2**32 blocks * 2**20 byte-sized blocks)
  offset @1 :UInt32;  # Offset within the block
  path @2 :Text;      # Relative path of the file
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
