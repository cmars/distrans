# distrans

"The bytes must flow."

# Usage

`distrans get <dht key> [directory]` to download a posted file (defaults to current directory).

`distrans post <file>` will serve a file, displaying the dht key where it can be downloaded.

# TODOs

## What is currently broken?

To distribute a large 4GB Linux ISO effectively point-to-point, these need to be addressed:

- [ ] File indexing is slow, need to improve performance of the seeding scan on large files.
- [ ] Fetcher doesn't yet renegotiate a new private route if/when it drops.
- [ ] Fetcher could run with a much higher rate of concurrency. Stress testing indicates Veilid will tolerate up to 20 fetchers.
- [ ] Verification of piece digests
- [ ] Resuming fetch where we left off, keeping track of state

# And then?

Full duplex peer downloading and uploading.

Trackers and swarms.

# Development

Currently only NixOS is supported. Cross-platform binaries and OCI images will come later.

`nix develop` to get a devshell, then

`cargo build`
