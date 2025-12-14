#!/bin/bash
# scripts/ntx-e2e-echo.sh
# 
# 完整的 Echo 场景端到端自动化脚本
# 
# 用法：
#   sudo ./scripts/ntx-e2e-echo.sh [--help] [--no-cleanup] [--tcpdump]
#
# 示例：
#   sudo ./scripts/ntx-e2e-echo.sh --tcpdump

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# 配置
HOST_TIMEOUT=30
CLIENT_TIMEOUT=15
POLL_INTERVAL=0.5
NO_CLEANUP=0
TCPDUMP_ENABLED=0
TCPDUMP_FILE=""

COMPONENT_PATH="plugins/scheduler/wac/echo_composed.wasm"
HOST_LOG="/tmp/ntx-echo-host.log"
CLIENT_LOG="/tmp/ntx-echo-client.log"
TCPDUMP_LOG="/tmp/ntx-echo-tcpdump.pcap"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}[info]${NC} $@"
}

log_ok() {
    echo -e "${GREEN}[✓]${NC} $@"
}

log_err() {
    echo -e "${RED}[✗]${NC} $@"
}

log_warn() {
    echo -e "${YELLOW}[!!]${NC} $@"
}

print_help() {
    cat <<EOF
用法: sudo $0 [OPTIONS]

选项:
  --help              显示此帮助信息
  --no-cleanup        不清理 veth 拓扑（默认运行结束后清理）
  --tcpdump           启用 tcpdump 抓包（保存到 $TCPDUMP_LOG）

示例:
  sudo $0
  sudo $0 --tcpdump
  sudo $0 --no-cleanup --tcpdump

输出日志:
  Host 日志:   $HOST_LOG
  Client 日志: $CLIENT_LOG
  tcpdump:    $TCPDUMP_LOG (如果启用)

EOF
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --help)
                print_help
                exit 0
                ;;
            --no-cleanup)
                NO_CLEANUP=1
                shift
                ;;
            --tcpdump)
                TCPDUMP_ENABLED=1
                shift
                ;;
            *)
                log_err "未知选项: $1"
                print_help
                exit 1
                ;;
        esac
    done
}

cleanup() {
    if [[ $NO_CLEANUP -eq 0 ]]; then
        log_info "清理 veth 拓扑..."
        sudo ip netns delete ntx1 2>/dev/null || true
        sudo ip netns delete ntx2 2>/dev/null || true
        sudo ip link delete veth0 2>/dev/null || true
    else
        log_info "保留 veth 拓扑（--no-cleanup 已启用）"
    fi
}

trap cleanup EXIT

main() {
    log_info "=========================================="
    log_info "NIC-HOST-Guest Echo 场景端到端测试"
    log_info "=========================================="

    # 0. 检查依赖
    log_info "检查依赖..."
    cd "$REPO_ROOT"
    
    if [[ ! -f "target/debug/Ntx" ]]; then
        log_err "找不到 target/debug/Ntx，请先运行 cargo build"
        exit 1
    fi
    
    if [[ ! -f "target/debug/examples/traffic-send" ]]; then
        log_err "找不到 traffic-send，请先运行 cargo build --examples"
        exit 1
    fi
    
    if [[ ! -f "$COMPONENT_PATH" ]]; then
        log_warn "未找到组件: $COMPONENT_PATH"
        log_info "尝试构建组件..."
        if [[ -f "plugins/scheduler/scripts/build_all_components.sh" ]]; then
            bash plugins/scheduler/scripts/build_all_components.sh || {
                log_err "组件构建失败，使用默认路径继续"
            }
        fi
    fi

    # 1. 设置网络拓扑
    log_info "设置 veth 拓扑..."
    if [[ ! -f "scripts/ntx-veth-up.sh" ]]; then
        log_err "找不到 scripts/ntx-veth-up.sh"
        exit 1
    fi
    
    bash scripts/ntx-veth-up.sh > /dev/null 2>&1 || {
        log_warn "veth 拓扑设置可能失败，继续..."
    }

    # 验证拓扑
    if ! ip netns show | grep -q ntx1; then
        log_err "veth 拓扑未正确设置"
        exit 1
    fi
    log_ok "veth 拓扑已就绪"

    # 2. 启动 tcpdump（如果启用）
    if [[ $TCPDUMP_ENABLED -eq 1 ]]; then
        log_info "启动 tcpdump..."
        rm -f "$TCPDUMP_LOG"
        tcpdump -i veth0 -n "udp port 10001" -w "$TCPDUMP_LOG" > /dev/null 2>&1 &
        TCPDUMP_PID=$!
        sleep 1
        log_ok "tcpdump PID: $TCPDUMP_PID"
    fi

    # 3. 启动 Host-1（Server）
    log_info "启动 Host-1（Server）..."
    rm -f "$HOST_LOG"

    timeout $HOST_TIMEOUT bash -c "
        sudo ./scripts/ntxns1.sh \
            ./target/debug/Ntx \
            --mode net \
            --iface ntx0 \
            --backend afpacket-dgram \
            --port 10001 \
            --component '$COMPONENT_PATH' \
            2>&1 | tee '$HOST_LOG'
    " > /dev/null 2>&1 &
    HOST_PID=$!
    log_ok "Host-1 PID: $HOST_PID"

    # 等待 Host-1 就绪
    log_info "等待 Host-1 就绪（最多 5 秒）..."
    for i in {1..10}; do
        if grep -q "listening on" "$HOST_LOG" 2>/dev/null; then
            log_ok "Host-1 已就绪"
            break
        fi
        sleep 0.5
        if [[ $i -eq 10 ]]; then
            log_warn "Host-1 启动超时，继续..."
        fi
    done

    # 4. 启动 Host-2（Client）
    log_info "启动 Host-2（Client）..."
    rm -f "$CLIENT_LOG"

    if timeout $CLIENT_TIMEOUT bash -c "
        sudo ./scripts/ntxns2.sh \
            ./target/debug/examples/traffic-send \
            --iface ntx1 \
            --backend afpacket-dgram \
            --dst-ips 10.0.0.1 \
            --src-ip 10.0.0.2 \
            --dst-port 10001 \
            --src-port 40000 \
            --rr \
            --pps 50 \
            --count 20 \
            2>&1 | tee '$CLIENT_LOG'
    "; then
        CLIENT_EXIT=0
        log_ok "Host-2 执行完成"
    else
        CLIENT_EXIT=$?
        log_warn "Host-2 执行异常或超时（exit: $CLIENT_EXIT）"
    fi

    # 5. 验证结果
    log_info "验证结果..."
    
    HOST_RX_UDP=$(grep "rx_udp" "$HOST_LOG" 2>/dev/null | tail -1 | grep -oP 'rx_udp=\K[0-9]+' || echo "0")
    HOST_TX_REPLIES=$(grep "tx_replies" "$HOST_LOG" 2>/dev/null | tail -1 | grep -oP 'tx_replies=\K[0-9]+' || echo "0")
    
    CLIENT_MATCHED=$(grep "final:" "$CLIENT_LOG" 2>/dev/null | grep -oP 'matched=\K[0-9]+' || echo "0")
    CLIENT_SENT=$(grep "final:" "$CLIENT_LOG" 2>/dev/null | grep -oP 'sent=\K[0-9]+' || echo "0")

    log_info ""
    log_info "========== 测试结果 =========="
    log_info "Host-1:"
    log_info "  rx_udp=$HOST_RX_UDP, tx_replies=$HOST_TX_REPLIES"
    log_info "Host-2:"
    log_info "  sent=$CLIENT_SENT, matched=$CLIENT_MATCHED, exit=$CLIENT_EXIT"

    # 判断成功/失败
    if [[ $HOST_RX_UDP -gt 0 ]] && [[ $HOST_TX_REPLIES -gt 0 ]] && [[ $CLIENT_MATCHED -gt 0 ]]; then
        log_ok "========== 测试 PASSED =========="
        TEST_PASSED=0
    else
        log_err "========== 测试 FAILED =========="
        TEST_PASSED=1
    fi

    # 6. 清理进程
    log_info "清理进程..."
    kill $HOST_PID 2>/dev/null || true
    
    if [[ $TCPDUMP_ENABLED -eq 1 ]]; then
        kill $TCPDUMP_PID 2>/dev/null || true
        if [[ -f "$TCPDUMP_LOG" ]]; then
            log_ok "tcpdump 已保存: $TCPDUMP_LOG"
            log_info "查看命令: tcpdump -r $TCPDUMP_LOG -X"
        fi
    fi

    # 7. 输出日志
    log_info ""
    log_info "========== 详细日志 =========="
    
    if [[ -s "$HOST_LOG" ]]; then
        log_info "Host-1 日志（最后 20 行）:"
        tail -20 "$HOST_LOG" | sed 's/^/  /'
    else
        log_warn "Host-1 日志为空或不存在"
    fi

    log_info ""
    if [[ -s "$CLIENT_LOG" ]]; then
        log_info "Host-2 日志（最后 20 行）:"
        tail -20 "$CLIENT_LOG" | sed 's/^/  /'
    else
        log_warn "Host-2 日志为空"
    fi

    # 返回状态
    exit $TEST_PASSED
}

# 执行
parse_args "$@"
main
