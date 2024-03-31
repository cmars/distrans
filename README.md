# distrans

[![crates.io/crates/distrans](https://img.shields.io/crates/v/distrans.svg)](https://crates.io/crates/distrans)
[![docs.rs/distrans-peer](https://img.shields.io/docsrs/distrans_peer)](https://docs.rs/distrans-peer)
[![MPL-2.0](https://img.shields.io/crates/l/distrans.svg)](./LICENSE)

"The bytes must flow."

Distrans (distribution and transfer) sends and receives file content anonymously over the [Veilid](https://veilid.com) network.

# Install

## Install binary release

Install a [binary release](https://github.com/cmars/distrans/releases) on Linux, macOS or Windows.

## Install Rust crate

Install the crate with `cargo install distrans`.

Crate dependencies may require system packages to be installed depending on the target platform.

Debian Bookworm needs `apt-get install build-essential libssl-dev pkg-config`.

Others may be similar.

## Install Nix flake

Run with `nix run github:cmars/distrans`.

Add this flake (`github:cmars/distrans`) as an input to your home manager flake.

Or add the default package to a legacy `home.nix` with something like:

    (builtins.getFlake "github:cmars/distrans").packages.x86_64-linux.default

# Usage

`distrans post <file>` indexes and seeds a file, displaying the dht key where it can be downloaded.

`distrans get <dht key> [directory]` downloads a posted file (defaults to current directory).

See `distrans --help` for more options.

# Development

In a Nix environment, `nix develop github:cmars/distrans` (or `nix develop` in this directory) to get a devshell with the necessary tool dependencies.

On other platforms a [Veilid development environment](https://gitlab.com/veilid/veilid/-/blob/2ec00e18da999dd16b8c84444bb1e60f9503e752/DEVELOPMENT.md) will suffice.

`capnp` is only necessary when working on the protocol wire-format.

## CICD

Github is used for CICD and especially [release automation](https://blog.orhun.dev/automated-rust-releases/).

## Contributions

Branches and releases are regularly mirrored to [Gitlab](https://gitlab.com/cmars232/distrans). Pull requests might be accepted from either, if they fit with the project plans and goals.

Open an issue and ask before picking up a branch and proposing, for best results.

# Roadmap

See [project plans](https://github.com/users/cmars/projects/1/views/1) for short-term plans and contribution ideas.

Long-term: Full duplex peer downloading and uploading. Trackers and swarms.
