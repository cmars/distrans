# distrans

![Crates.io Version](https://img.shields.io/crates/v/distrans)

"The bytes must flow."

Distrans (distribution and transfer) sends and receives file content anonymously over the [Veilid](https://veilid.com) network.

# Usage

`distrans post <file>` indexes and seeds a file, displaying the dht key where it can be downloaded.

`distrans get <dht key> [directory]` downloads a posted file (defaults to current directory).

# Roadmap

See [project plans](https://github.com/users/cmars/projects/1/views/1) for short-term plans and contribution ideas.

Long-term: Full duplex peer downloading and uploading. Trackers and swarms.

# Development

Currently distrans builds against veilid-core 0.2.5, which at time of writing does not build without serde tooling installed. Specific versions of capnproto and protobuf are required. First set up a [development environment for Veilid](https://gitlab.com/veilid/veilid/-/blob/main/DEVELOPMENT.md#veilid-development) on your platform of choice (Android / Flutter stuff not required).

Then `cargo install distrans` and other `cargo` commands should work.

## NixOS

The original author of this project develops on NixOS. If you Nix too,

### Developing

`nix develop` in here to get a devshell, then

`cargo build` and other `cargo` commands to do things.

### Running

`nix run github:cmars/distrans`

## CICD

Github is used for CICD and especially [release automation](https://blog.orhun.dev/automated-rust-releases/).

## Contributions

Branches and releases are regularly mirrored to [Gitlab](https://gitlab.com/cmars232/distrans). Pull requests might be accepted from either, if they fit with the project plans and goals.

Open an issue and ask before picking up a branch and proposing, for best results.
