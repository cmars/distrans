# distrans

"The bytes must flow."

# Preview

Not implemented yet but coming soon:

`distrans get <dht key> <file>` to download a posted file.

`distrans post <file>` will serve a file, displaying the dht key where it can be downloaded.

# And then?

Full duplex peer downloading and uploading.

Trackers and swarms.

# Development

Currently only NixOS is supported. Cross-platform binaries and OCI images will come later.

`nix develop` to get a devshell, then

`cargo build`
