fn main() {
    // Compile Capn'Proto
    ::capnpc::CompilerCommand::new()
        .output_path("src/")
        .default_parent_module(vec!["proto".into()])
        .file("proto/distrans.capnp")
        .run()
        .expect("compiling schema");
}
