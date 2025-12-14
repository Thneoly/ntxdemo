#!/bin/bash
# 
# Echo 场景完整编译脚本
# 
# 功能：
# 1. 编译所有组件（corelibs、eventbus、actions-executor-server、actions-executor-client、scheduler）
# 2. 使用 WAC 编排生成 echo-server.wasm 和 echo-client.wasm
# 3. 输出最终的 .wasm 文件
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCHEDULER_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$SCHEDULER_DIR/.." && pwd)"

echo "╔════════════════════════════════════════════════════════════╗"
echo "║          Echo 场景 WAC 编排编译脚本                        ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""

# 检查 wac 工具
if ! command -v wac &> /dev/null; then
    echo "[!] Error: 'wac' tool not found. Please install it:"
    echo "    https://github.com/bytecodealliance/wac"
    exit 1
fi

echo "[*] Compilation environment:"
echo "    Project root: $PROJECT_ROOT"
echo "    Scheduler dir: $SCHEDULER_DIR"
echo ""

# 1. 编译 corelibs
echo "[1/5] Building core-libs..."
cd "$SCHEDULER_DIR/core-libs"
cargo build --target wasm32-wasip2 --release 2>&1 | grep -E "Compiling|Finished|error" || true
CORELIBS_PATH="$SCHEDULER_DIR/core-libs/target/wasm32-wasip2/release/scheduler_core_libs.wasm"
if [ ! -f "$CORELIBS_PATH" ]; then
    echo "[!] Error: Failed to build core-libs"
    exit 1
fi
echo "    ✓ core-libs compiled: $(stat -c%s "$CORELIBS_PATH" 2>/dev/null || stat -f%z "$CORELIBS_PATH") bytes"

# 2. 编译 eventbus
echo "[2/5] Building eventbus..."
cd "$SCHEDULER_DIR/eventbus"
cargo build --target wasm32-wasip2 --release 2>&1 | grep -E "Compiling|Finished|error" || true
EVENTBUS_PATH="$SCHEDULER_DIR/eventbus/target/wasm32-wasip2/release/scheduler_eventbus.wasm"
if [ ! -f "$EVENTBUS_PATH" ]; then
    echo "[!] Error: Failed to build eventbus"
    exit 1
fi
echo "    ✓ eventbus compiled: $(stat -c%s "$EVENTBUS_PATH" 2>/dev/null || stat -f%z "$EVENTBUS_PATH") bytes"

# 3. 编译 actions-executor-server
echo "[3/5] Building actions-executor-server..."
cd "$SCHEDULER_DIR/actions-executor-server"
cargo build --target wasm32-wasip2 --release 2>&1 | grep -E "Compiling|Finished|error" || true
SERVER_ACTIONS_PATH="$SCHEDULER_DIR/actions-executor-server/target/wasm32-wasip2/release/scheduler_actions_executor_server.wasm"
if [ ! -f "$SERVER_ACTIONS_PATH" ]; then
    echo "[!] Error: Failed to build actions-executor-server"
    exit 1
fi
echo "    ✓ actions-executor-server compiled: $(stat -c%s "$SERVER_ACTIONS_PATH" 2>/dev/null || stat -f%z "$SERVER_ACTIONS_PATH") bytes"

# 4. 编译 actions-executor-client
echo "[4/5] Building actions-executor-client..."
cd "$SCHEDULER_DIR/actions-executor-client"
cargo build --target wasm32-wasip2 --release 2>&1 | grep -E "Compiling|Finished|error" || true
CLIENT_ACTIONS_PATH="$SCHEDULER_DIR/actions-executor-client/target/wasm32-wasip2/release/scheduler_actions_executor_client.wasm"
if [ ! -f "$CLIENT_ACTIONS_PATH" ]; then
    echo "[!] Error: Failed to build actions-executor-client"
    exit 1
fi
echo "    ✓ actions-executor-client compiled: $(stat -c%s "$CLIENT_ACTIONS_PATH" 2>/dev/null || stat -f%z "$CLIENT_ACTIONS_PATH") bytes"

# 5. 编译 scheduler
echo "[5/5] Building scheduler..."
cd "$SCHEDULER_DIR/scheduler"
cargo build --target wasm32-wasip2 --release 2>&1 | grep -E "Compiling|Finished|error" || true
SCHEDULER_PATH="$SCHEDULER_DIR/scheduler/target/wasm32-wasip2/release/scheduler.wasm"
if [ ! -f "$SCHEDULER_PATH" ]; then
    echo "[!] Error: Failed to build scheduler"
    exit 1
fi
echo "    ✓ scheduler compiled: $(stat -c%s "$SCHEDULER_PATH" 2>/dev/null || stat -f%z "$SCHEDULER_PATH") bytes"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "[*] WAC 编排阶段"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# 6. WAC 编排：echo-server
echo "[*] Composing echo-server.wasm..."
cd "$SCHEDULER_DIR/wac"
wac plug echo-server.wac -o echo-server.wasm
SERVER_WASM="$SCHEDULER_DIR/wac/echo-server.wasm"
if [ ! -f "$SERVER_WASM" ]; then
    echo "[!] Error: Failed to compose echo-server.wasm"
    exit 1
fi
echo "    ✓ echo-server.wasm: $(stat -c%s "$SERVER_WASM" 2>/dev/null || stat -f%z "$SERVER_WASM") bytes"

# 7. WAC 编排：echo-client
echo "[*] Composing echo-client.wasm..."
wac plug echo-client.wac -o echo-client.wasm
CLIENT_WASM="$SCHEDULER_DIR/wac/echo-client.wasm"
if [ ! -f "$CLIENT_WASM" ]; then
    echo "[!] Error: Failed to compose echo-client.wasm"
    exit 1
fi
echo "    ✓ echo-client.wasm: $(stat -c%s "$CLIENT_WASM" 2>/dev/null || stat -f%z "$CLIENT_WASM") bytes"

echo ""
echo "╔════════════════════════════════════════════════════════════╗"
echo "║                    编译完成！                              ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""
echo "[+] 输出文件："
echo "    • $SERVER_WASM"
echo "    • $CLIENT_WASM"
echo ""
echo "[*] 下一步："
echo "    1. 在 Host-1 上运行 echo-server："
echo "       ./target/release/Ntx --mode server --iface eth0 --component $SERVER_WASM"
echo ""
echo "    2. 在 Host-2 上运行 echo-client："
echo "       ./target/release/Ntx --mode client --iface eth1 --component $CLIENT_WASM"
echo ""
