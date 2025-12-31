#!/bin/bash
set -e
pushd actions-executor
    cargo build --target wasm32-wasip2
popd

pushd eventbus
    cargo build --target wasm32-wasip2
popd

pushd scheduler
    cargo build --target wasm32-wasip2
popd

rm -rf wac/deps/component/*.wasm
mkdir -p wac/deps/component/
cp ../target/wasm32-wasip2/debug/actions_executor.wasm wac/deps/component/actions-executor.wasm
cp ../target/wasm32-wasip2/debug/eventbus.wasm wac/deps/component/eventbus.wasm
cp ../target/wasm32-wasip2/debug/scheduler.wasm wac/deps/component/scheduler.wasm

wac compose wac/scheduler-composition.wac   --deps-dir wac/deps   -o wac/scheduler-composed.wasm

wasm-tools component wit wac/scheduler-composed.wasm  | head -n 40