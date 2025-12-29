#!/bin/bash
cargo run -p actions-catalog-gen -- \
	target/wasm32-wasip2/debug/actions_executor.wasm \
	component/conf/udp-echo-minimal/actions-catalog.json