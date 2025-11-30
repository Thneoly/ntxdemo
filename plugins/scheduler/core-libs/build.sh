#!/usr/bin/env bash
set -e

cd "$(dirname "$0")"

echo "Building scheduler-core as wasm32-wasip2 component..."

cargo component build --target wasm32-wasip2 --release

echo "✓ scheduler-core component built successfully"
echo "  Output: target/wasm32-wasip2/release/scheduler_core.wasm"
