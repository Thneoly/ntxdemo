#!/bin/bash
cargo build --target=wasm32-wasip2
cp ./target/wasm32-wasip2/debug/core.wasm ./core.wasm
WASMTIME_BACKTRACE_DETAILS=1  wasmtime run -S tcp=y -S inherit-network=y --invoke="start()" ./core.wasm