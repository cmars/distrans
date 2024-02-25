#!/usr/bin/env bash

set -eux

build_dir=$(mktemp -d)
trap "rm -rf $build_dir" EXIT

brew install capnp

UNAME_M=$(uname -m)
if [[ "$UNAME_M" == "x86_64" ]]; then 
    PROTOC_ARCH=x86_64
elif [[ "$UNAME_M" == "aarch64" ]]; then 
    PROTOC_ARCH=aarch_64
else 
    echo Unsupported build architecture
    exit 1
fi 
PROTOC_OS="osx"

curl -OL https://github.com/protocolbuffers/protobuf/releases/download/v$PROTOC_VERSION/protoc-$PROTOC_VERSION-$PROTOC_OS-$PROTOC_ARCH.zip
unzip protoc-$PROTOC_VERSION-$PROTOC_OS-$PROTOC_ARCH.zip
chmod +x bin/*
sudo cp -r bin/* /usr/local/bin/
sudo cp -r include/* /usr/local/include/

capnp --version
protoc --version
