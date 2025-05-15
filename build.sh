#!/usr/bin/env bash

# exit on fail
set -xe

# server
cargo build -p reamioserver

# client
pushd ui/
trunk build
popd

# merge
rm -r devdir/dist
cp -r ui/dist/ devdir/dist
