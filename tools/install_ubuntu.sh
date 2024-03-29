#!/usr/bin/env bash

set -eux

build_dir=$(mktemp -d)
trap "rm -rf $build_dir" EXIT
cd $build_dir

# Enable brew
eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"

brew install capnp
CAPNP=$(which capnp)

PROTOC_VERSION="24.3" # Keep in sync with veilid-core/build.rs

# Install protoc
UNAME_M=$(uname -m)
if [[ "$UNAME_M" == "x86_64" ]]; then 
    PROTOC_ARCH=x86_64
elif [[ "$UNAME_M" == "aarch64" ]]; then 
    PROTOC_ARCH=aarch_64
else 
    echo Unsupported build architecture
    exit 1
fi 
PROTOC_OS="linux"

curl -OL https://github.com/protocolbuffers/protobuf/releases/download/v$PROTOC_VERSION/protoc-$PROTOC_VERSION-$PROTOC_OS-$PROTOC_ARCH.zip
unzip protoc-$PROTOC_VERSION-$PROTOC_OS-$PROTOC_ARCH.zip
chmod +x bin/*
sudo cp -r bin/* /usr/local/bin/
sudo cp -r include/* /usr/local/include/

sudo ln -s $CAPNP /usr/local/bin

capnp --version
protoc --version
