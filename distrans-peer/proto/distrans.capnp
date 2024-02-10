@0xc46f97c2b79df618;

# Logical payload Metadata

struct SHA256 {
  p0 @0 :UInt64;
  p1 @1 :UInt64;
  p2 @2 :UInt64;
  p3 @3 :UInt64;
}

const pieceMaxLength: UInt32 = 1048576;

struct Payload {
  # Metadata about the entire payload to be delivered.

  digest @0 :SHA256;
  length @1 :UInt64;
}

struct Piece {
  # Metadata about a piece of the payload.
  # Pieces are 1MB in size max.

  digest @0 :SHA256;
  length @1 :UInt32 = .pieceMaxLength;
}

struct File {
  # Metadata about files which exist within the payload.

  contents @1 :Slice;  # Where the contents are located in the payload.
  path @0 :Text;       # A suggested name for the file.
}

struct Slice {
  # Describe a contiguous stream of bytes within a payload.
  # Pieces are consumed in consecutive order from the starting
  # piece until the length is reached.

  startingPiece @0 :UInt32;
  pieceOffset @1 :UInt32;
  length @2 :UInt64;
}

# Transport-layer packing of the above into DHT constraints

struct PayloadRecordMain {
  # Main payload record containing the beginning of a payload index.
  # Smaller payloads might fit within a single subkey value.

  payload @0 :Payload;
  files @1 :List(File);
  pieces @2 :List(Piece);
}

struct PayloadRecordPieces {
  # Additional pieces, if they didn't all fit in the main record.
  # Pieces are to be appended in sequential order picking up from
  # where the prior subkey left off.

  pieces @0 :List(Piece);
}

struct PayloadRecordFiles {
  # Additional files, if they didn't all fit in the main record.
  # Files are to be appended in sequential order picking up from
  # where the prior subkey left off.

  pieces @0 :List(Piece);
}
