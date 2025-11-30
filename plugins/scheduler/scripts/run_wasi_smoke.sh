#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname "$0")" && pwd)"
SCHEDULER_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd -- "$SCHEDULER_DIR/../.." && pwd)"

pushd "$SCHEDULER_DIR" >/dev/null

# Build WASM component and host http_server binary
cargo build --target wasm32-wasip2 --lib
cargo build --bin http_server

COMPONENT_PATH="$SCHEDULER_DIR/target/wasm32-wasip2/debug/scheduler.wasm"
if [[ ! -f "$COMPONENT_PATH" ]]; then
  echo "scheduler component not found at $COMPONENT_PATH" >&2
  exit 1
fi

TARGET_HTTP_SERVER="$SCHEDULER_DIR/target/debug/http_server"
if [[ ! -x "$TARGET_HTTP_SERVER" ]]; then
  echo "http_server binary not found at $TARGET_HTTP_SERVER" >&2
  exit 1
fi

# Launch demo HTTP server
SERVER_LOG="/tmp/http_server_wasi_smoke.log"
"$TARGET_HTTP_SERVER" >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID >/dev/null 2>&1 || true' EXIT
sleep 1

if ! grep -q "HTTP test server listening" "$SERVER_LOG"; then
  echo "http_server failed to start; log:" >&2
  cat "$SERVER_LOG" >&2
  exit 1
fi

popd >/dev/null

pushd "$REPO_ROOT" >/dev/null
SCHEDULER_COMPONENT="$COMPONENT_PATH" cargo run -- plugins/scheduler/res/simple_scenario.yaml
popd >/dev/null
