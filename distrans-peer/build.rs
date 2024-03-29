use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::Write;
use std::{io, path::Path};

// Adapted from https://gitlab.com/veilid/veilid, veilid-core/build.rs
// which is released by the Veilid Team <contact@veilid.com> under the MPL-2.0.

fn is_input_file_outdated<P, Q>(input: P, output: Q) -> io::Result<bool>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let Some(out_bh) = get_build_hash(output) else {
        // output file not found or no build hash, we are outdated
        return Ok(true);
    };

    let in_bh = make_build_hash(input);

    Ok(out_bh != in_bh)
}

fn calculate_hash(lines: std::io::Lines<std::io::BufReader<std::fs::File>>) -> Vec<u8> {
    let mut hasher = Sha256::new();
    // Build hash of lines, ignoring EOL conventions
    for l in lines {
        let l = l.unwrap();
        hasher.update(l.as_bytes());
        hasher.update(b"\n");
    }
    let out = hasher.finalize();
    out.to_vec()
}

fn get_build_hash<Q: AsRef<Path>>(output_path: Q) -> Option<Vec<u8>> {
    let lines = std::io::BufReader::new(std::fs::File::open(output_path).ok()?).lines();
    for l in lines {
        let l = l.unwrap();
        if let Some(rest) = l.strip_prefix("//BUILDHASH:") {
            return Some(hex::decode(rest).unwrap());
        }
    }
    None
}

fn make_build_hash<P: AsRef<Path>>(input_path: P) -> Vec<u8> {
    let input_path = input_path.as_ref();
    let lines = std::io::BufReader::new(std::fs::File::open(input_path).unwrap()).lines();
    calculate_hash(lines)
}

fn append_hash<P: AsRef<Path>, Q: AsRef<Path>>(input_path: P, output_path: Q) {
    let input_path = input_path.as_ref();
    let output_path = output_path.as_ref();
    let lines = std::io::BufReader::new(std::fs::File::open(input_path).unwrap()).lines();
    let h = calculate_hash(lines);
    let mut out_file = OpenOptions::new().append(true).open(output_path).unwrap();
    writeln!(out_file, "\n//BUILDHASH:{}", hex::encode(h)).unwrap();
}

fn main() {
    if std::env::var("DOCS_RS").is_ok()
        || std::env::var("CARGO_CFG_DOC").is_ok()
        || std::env::var("BUILD_DOCS").is_ok()
    {
        return;
    }

    if is_input_file_outdated("./proto/distrans.capnp", "./src/proto/distrans_capnp.rs").unwrap() {
        // Compile Capn'Proto
        ::capnpc::CompilerCommand::new()
            .output_path("src/")
            .default_parent_module(vec!["proto".into()])
            .file("proto/distrans.capnp")
            .run()
            .expect("compiling schema");
        // If successful, append a hash of the input to the output file
        append_hash("proto/distrans.capnp", "src/proto/distrans_capnp.rs");
    }
}
