# distrans

[![crates.io/crates/distrans](https://img.shields.io/crates/v/distrans.svg)](https://crates.io/crates/distrans)
[![docs.rs/distrans-peer](https://img.shields.io/docsrs/distrans_peer)](https://docs.rs/distrans-peer)
[![MPL-2.0](https://img.shields.io/crates/l/distrans.svg)](./LICENSE)

"The bytes must flow."

Distrans (distribution and transfer) sends and receives file content anonymously over the [Veilid](https://veilid.com) network.

# Usage

`distrans seed <file>` indexes and seeds a file , displaying the dht key which can be used to fetch it.

`distrans fetch <dht key> [directory]` fetches a file while it's being seeded (defaults to current directory).

Similar to bittorrent, distrans cannot fetch a file unless it's being seeded by a peer.

See `distrans --help` for more options.

## Try it!

Try fetching a test file with `distrans fetch VLD0:cCHB85pEaV4bvRfywxnd2fRNBScR64UaJC8hoKzyr3M`.

# Install

Install a [binary release](https://github.com/cmars/distrans/releases) on Linux, macOS or Windows.

## Rust crate

Install the crate with `cargo install distrans`.

Crate dependencies may require system packages to be installed depending on the target platform.

Debian Bookworm needs `apt-get install build-essential libssl-dev pkg-config`.

Others may be similar.

## Nix flake

Run distrans on Nix with `nix run github:cmars/distrans`.

Add this flake (`github:cmars/distrans`) as an input to your home manager flake.

Or add the default package to a legacy `home.nix` with something like:

    (builtins.getFlake "github:cmars/distrans").packages.x86_64-linux.default

# Plans

What's on the roadmap for a 1.0 release.

## Trackers

Trackers will enable a swarm of distrans peers to operate more like bittorrent, where blocks may be simultaneously seeded and fetched.

## Multi-file shares

The distrans wire protocol and indexing structures support multi-file shares, but this hasn't been fully implemented yet.

# Troubleshooting

## Clock skew

Distrans operates an embedded Veilid node, which requires a synchronized local clock. Clock skew can prevent distrans from connecting to the Veilid network.

## Debug logging

Logging can be configured with the [RUST_LOG environment variable](https://docs.rs/env_logger/latest/env_logger/#enabling-logging).

`RUST_LOG=debug` will enable all debug-level logging in distrans as well as veilid-core, which may be useful for troubleshooting low-level Veilid network problems and reporting issues.

## Issues

When opening an issue, note the OS type, OS release version, distrans version, and steps to reproduce. Any logs you can attach may help.

# Development

In a Nix environment, `nix develop github:cmars/distrans` (or `nix develop` in this directory) to get a devshell with the necessary tool dependencies.

On other platforms a [Veilid development environment](https://gitlab.com/veilid/veilid/-/blob/2ec00e18da999dd16b8c84444bb1e60f9503e752/DEVELOPMENT.md) will suffice.

`capnp` is only necessary when working on the protocol wire-format.

## CICD

Github is used for CICD and especially [release automation](https://blog.orhun.dev/automated-rust-releases/).

## Contributions

Branches and releases are regularly mirrored to [Gitlab](https://gitlab.com/cmars232/distrans). Pull requests might be accepted from either, if they fit with the project plans and goals.

Open an issue and ask before picking up a branch and proposing, for best results.

