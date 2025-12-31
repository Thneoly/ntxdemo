#!/bin/bash
set -e
cargo build -p actions-executor --target wasm32-wasip2
cp target/wasm32-wasip2/debug/actions_executor.wasm scripts/oras/
pushd scripts/oras
  cargo run -p actions-catalog-gen -- actions_executor.wasm actions-catalog.json
  oras login --ca-file /home/cc/Desktop/harbor/certs/harbor.crt  -u admin -p Harbor12345 192.168.31.138
  oras push --ca-file=/home/cc/Desktop/harbor/certs/harbor.crt   192.168.31.138/ntx/executor:v0.0.1 \
    --artifact-type application/vnd.ntx.action-executor.v1  actions_executor.wasm:application/wasm \
    actions-catalog.json:application/json

popd