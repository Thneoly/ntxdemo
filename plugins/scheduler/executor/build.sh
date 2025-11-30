#!/usr/bin/env bash
set -e

cd "$(dirname "$0")"

echo "Building scheduler-executor as wasm32-wasip2 component..."

cargo component build --target wasm32-wasip2 --release

echo "✓ scheduler-executor component built successfully"
echo "  Output: target/wasm32-wasip2/release/scheduler_executor.wasm"
