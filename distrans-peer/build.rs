fn main() {
    if std::env::var("DOCS_RS").is_ok()
        || std::env::var("CARGO_CFG_DOC").is_ok()
        || std::env::var("BUILD_DOCS").is_ok()
    {
        return;
    }

    // Compile Capn'Proto
    ::capnpc::CompilerCommand::new()
        .output_path("src/")
        .default_parent_module(vec!["proto".into()])
        .file("proto/distrans.capnp")
        .run()
        .expect("compiling schema");
}
