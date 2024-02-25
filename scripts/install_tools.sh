#!/usr/bin/env bash

set -eux

# TODO: support windows somehow
UNAME_S=$(uname -s)

CAPNP_VERSION="1.0.2"
PROTOC_VERSION="24.3" # Keep in sync with veilid-core/build.rs

build_dir=$(mktemp -d)
trap "sudo rm -rf $build_dir" EXIT

cd $build_dir
mkdir -p capnp-build protoc-install

# Install capnp
pushd capnp-build
curl -O https://capnproto.org/capnproto-c++-${CAPNP_VERSION}.tar.gz
tar zxf capnproto-c++-${CAPNP_VERSION}.tar.gz
cd capnproto-c++-1.0.2
./configure
sudo make -j6 install
popd

# Install protoc
pushd protoc-install
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
if [[ "$UNAME_S" == "Linux" ]]; then
    PROTOC_OS=linux
elif [[ "$UNAME_S" == "Darwin" ]]; then
    PROTOC_OS=osx
else
    echo Unsupported OS $UNAME_S
    exit 1
fi

curl -OL https://github.com/protocolbuffers/protobuf/releases/download/v$PROTOC_VERSION/protoc-$PROTOC_VERSION-$PROTOC_OS-$PROTOC_ARCH.zip
unzip protoc-$PROTOC_VERSION-$PROTOC_OS-$PROTOC_ARCH.zip
chmod +x bin/*
sudo cp -r bin/* /usr/local/bin/
sudo cp -r include/* /usr/local/include/
popd

/usr/local/bin/capnp --version
/usr/local/bin/protoc --version
