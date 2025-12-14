#!/bin/bash
# 
# Echo 场景快速测试脚本 v2 (简化版本)
# 
# 功能：
# 1. 编译 Host 程序
# 2. 运行 echo-server 模式测试
# 3. 验证基本功能
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCHEDULER_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$SCHEDULER_DIR/../.." && pwd)"

echo "╔════════════════════════════════════════════════════════════╗"
echo "║          Echo 场景快速测试脚本 v2                          ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""

echo "[*] Environment:"
echo "    Project root: $PROJECT_ROOT"
echo "    Scheduler dir: $SCHEDULER_DIR"
echo ""

# 1. 编译 Host 程序
echo "[1/3] Building Host program (Ntx)..."
cd "$PROJECT_ROOT"
cargo build --bin Ntx 2>&1 | grep -E "Compiling ntx|Compiling Ntx|Finished|error" || true

if [ ! -f "$PROJECT_ROOT/target/debug/Ntx" ]; then
    echo "[!] Error: Failed to build Ntx"
    exit 1
fi
echo "    ✓ Ntx built successfully"
echo ""

# 2. 测试 Server 模式的帮助信息
echo "[2/3] Testing server mode..."
if "$PROJECT_ROOT/target/debug/Ntx" --help 2>&1 | grep -q "server\|client"; then
    echo "    ✓ Server mode help available"
else
    echo "[!] Warning: Server mode not found in help"
fi
echo ""

# 3. 显示快速启动命令
echo "[3/3] Setup and run commands:"
echo ""
echo "📋 Network topology setup (run once):"
echo "  sudo ip link add veth1 type veth peer name veth2"
echo "  sudo ip netns add ns1 2>/dev/null || true"
echo "  sudo ip netns add ns2 2>/dev/null || true"
echo "  sudo ip link set veth1 netns ns1"
echo "  sudo ip link set veth2 netns ns2"
echo "  sudo ip netns exec ns1 ip addr add 10.0.0.1/24 dev veth1"
echo "  sudo ip netns exec ns2 ip addr add 10.0.0.2/24 dev veth2"
echo "  sudo ip netns exec ns1 ip link set veth1 up"
echo "  sudo ip netns exec ns2 ip link set veth2 up"
echo ""

echo "🚀 Terminal 1 - Start Echo Server:"
echo "  sudo ip netns exec ns1 $PROJECT_ROOT/target/debug/Ntx \\"
echo "    --mode server --iface veth1 --port 10001"
echo ""

echo "🚀 Terminal 2 - Start Echo Client:"
echo "  sudo ip netns exec ns2 $PROJECT_ROOT/target/debug/Ntx \\"
echo "    --mode client --iface veth2 --server-ip 10.0.0.1 \\"
echo "    --server-port 10001 --count 100 --pps 50"
echo ""

echo "╔════════════════════════════════════════════════════════════╗"
echo "║                    Build complete! ✅                      ║"
echo "╚════════════════════════════════════════════════════════════╝"
