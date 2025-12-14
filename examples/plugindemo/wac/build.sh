#!/bin/bash
pushd ../demo
    ./build.sh
popd

pushd ../tcp-client
    ./build.sh
popd

pushd ../core
    ./build.sh
popd

cp ../demo/target/wasm32-wasip2/debug/demo.wasm ../core/target/wasm32-wasip2/debug/core.wasm ../tcp-client/target/wasm32-wasip2/debug/tcp_client.wasm ./

wac plug --plug core.wasm tcp_client.wasm -o mydemo.wasm
wac plug --plug mydemo.wasm demo.wasm -o mydemo.wasm