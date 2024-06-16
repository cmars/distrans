{
  description = "ddcp";

  inputs = {
    # NixOS 23.11 has recent enough versions of capnproto and protobuf to
    # develop on Veilid.
    nixpkgs.url = "github:nixos/nixpkgs/nixos-24.05";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };

    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane }:
    (flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            rust-overlay.overlays.default
          ];
        };

        arch = flake-utils.lib.system.system;

      in {

        devShells.default = pkgs.mkShell {
          buildInputs = (with pkgs; [
            cargo
            cargo-watch
            (rust-bin.stable.latest.default.override { extensions = [ "rust-src" ]; })
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

        packages.default = crane.lib.${system}.buildPackage {
          src = ./.;

          buildInputs = with pkgs; [
            cargo
            rust-bin.stable.latest.default
            capnproto
            protobuf
            pkg-config
            openssl
          ];
        };
      }
    ));
}
