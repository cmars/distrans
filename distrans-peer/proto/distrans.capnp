@0xc46f97c2b79df618;

# Logical payload Metadata

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
  payload @0 :Payload;
  pieces @1 :List(Piece);
  files @2 :List(File);
}

# Transport-layer packing of the above into DHT constraints

struct RecordHeader {
  subkeys @0 :UInt16;
  union {
    nextDhtKey @1 :Text;
    endOfRecord @2 :Void;
  }
}
