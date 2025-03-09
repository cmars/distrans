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

struct Header {
  # Describe the payload and how to get it.

  payload @0 :Payload;      # Identify the payload offered.
  subkeys @1 :UInt16;       # Number of DHT subkeys following this one; concatenate and decode to get the Index.
  route @2 :Data;           # Private route to request pieces from this peer.
}

###################################################################
# app_call protocol structures
###################################################################

struct BlockRequest {
  # Request a block

  piece @0 :UInt32;
  block @1 :UInt8;
}
