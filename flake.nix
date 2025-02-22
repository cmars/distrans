{
  description = "distrans";

  inputs = {
    # NixOS 24.11 has recent enough versions of capnproto and protobuf to
    # develop on Veilid.
    nixpkgs.url = "github:nixos/nixpkgs/nixos-24.11";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane }:
    (flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        craneLib = crane.mkLib pkgs;
        arch = flake-utils.lib.system.system;

      in {

        devShells.default = pkgs.mkShell {
          buildInputs = (with pkgs; [
            cargo
            cargo-watch
            (rust-bin.stable."1.81.0".default.override { extensions = [ "rust-src" ]; })
            rustfmt
            rust-analyzer
            clang
            llvmPackages.llvm
            llvmPackages.libclang
            gnumake
            cmake
            capnproto
            protobuf
            pkg-config
            openssl
            flyctl
          ]);

          LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib";
        };

        packages.default = craneLib.buildPackage {
          pname = "distrans";
          src = ./.;

          buildInputs = with pkgs; [
            cargo
            rust-bin.stable."1.81.0".default
            capnproto
            protobuf
            pkg-config
            openssl
          ];
        };
      }
    ));
}
