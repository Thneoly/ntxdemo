#!/bin/bash
./build.sh
WASMTIME_BACKTRACE_DETAILS=1  wasmtime run -S tcp=y -S inherit-network=y --invoke="start()" ./mydemo.wasm