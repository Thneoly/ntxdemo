#!/usr/bin/env bash
set -euo pipefail

# Generate actions catalog JSON for udp-echo-minimal.
#
# This script is meant for developer convenience and demo reproducibility.
# It builds the actions-executor WASIp2 component and then runs
# `actions-catalog-gen` to emit:
#   component/conf/udp-echo-minimal/actions-catalog.json

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT_DIR"

cargo build -p actions-executor --target wasm32-wasip2

cargo run -p actions-catalog-gen -- \
  target/wasm32-wasip2/debug/actions_executor.wasm \
  component/conf/udp-echo-minimal/actions-catalog.json

echo "Wrote component/conf/udp-echo-minimal/actions-catalog.json"