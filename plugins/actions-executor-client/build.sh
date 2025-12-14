#!/bin/bash
# 编译 Client 端的 actions-executor 组件

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "[*] Building actions-executor-client..."
cargo build --target wasm32-wasip2 "$@"

echo "[+] Build successful"
TARGET_PATH="target/wasm32-wasip2/debug/scheduler_actions_executor_client.wasm"
if [ -f "$TARGET_PATH" ]; then
    echo "    Output: $TARGET_PATH"
    echo "    Size: $(stat -f%z "$TARGET_PATH" 2>/dev/null || stat -c%s "$TARGET_PATH" 2>/dev/null) bytes"
else
    echo "    Warning: Expected output not found at $TARGET_PATH"
fi
