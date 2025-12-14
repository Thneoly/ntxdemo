#!/bin/bash
#
# Echo 场景功能测试脚本
# 模拟 Server 和 Client 的简单交互测试
#

set -e

PROJECT_ROOT="/home/cc/Desktop/code/GitHub/Ntx"
NTX_BIN="$PROJECT_ROOT/target/debug/Ntx"

echo "╔════════════════════════════════════════════════════════════╗"
echo "║            Echo 场景功能测试 - Server 模式                 ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""

# 1. 验证 Ntx 二进制文件
if [ ! -f "$NTX_BIN" ]; then
    echo "[!] Error: $NTX_BIN not found"
    exit 1
fi
echo "[✓] Ntx binary found: $NTX_BIN"
echo ""

# 2. 检查 veth 接口
echo "[*] Checking network interfaces..."
if sudo ip netns exec ns1 ip link show veth1 >/dev/null 2>&1; then
    echo "[✓] Network namespace ns1 with veth1 ready"
else
    echo "[!] Network namespace ns1 setup failed"
    exit 1
fi

if sudo ip netns exec ns2 ip link show veth2 >/dev/null 2>&1; then
    echo "[✓] Network namespace ns2 with veth2 ready"
else
    echo "[!] Network namespace ns2 setup failed"
    exit 1
fi
echo ""

# 3. 启动 Server（30秒超时）
echo "[*] Starting Echo Server in ns1..."
echo "    Command: timeout 30 sudo ip netns exec ns1 $NTX_BIN --mode server --iface veth1 --port 10001"
echo ""

timeout 30 sudo ip netns exec ns1 "$NTX_BIN" --mode server --iface veth1 --port 10001 &
SERVER_PID=$!

# 给 Server 一些时间启动
sleep 2

# 4. 启动 Client（从另一个命名空间）
echo "[*] Starting Echo Client in ns2..."
echo "    Command: timeout 10 sudo ip netns exec ns2 $NTX_BIN --mode client --iface veth2 --server-ip 10.0.0.1 --count 10"
echo ""

timeout 10 sudo ip netns exec ns2 "$NTX_BIN" --mode client --iface veth2 \
    --server-ip 10.0.0.1 --server-port 10001 --count 10 --pps 5 || true

echo ""
echo "╔════════════════════════════════════════════════════════════╗"
echo "║                    测试完成！                              ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""
echo "[📊] Next steps:"
echo "  1. Check Server mode output for packet statistics"
echo "  2. Verify Client mode packet generation"
echo "  3. Review 'docs/ECHO_QUICKSTART.md' for more details"
echo ""
