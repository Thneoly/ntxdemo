#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname "$0")" && pwd)"
SCHEDULER_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd -- "$SCHEDULER_DIR/../.." && pwd)"
COMPONENT_PATH="$SCHEDULER_DIR/target/wasm32-wasip2/debug/scheduler.wasm"
HTTP_SERVER_BIN="$SCHEDULER_DIR/target/debug/http_server"
SERVER_LOG="${SERVER_LOG:-/tmp/http_server_wasi_manual.log}"
RUN_LOG="${RUN_LOG:-/tmp/wasi_simple_scenario.log}"

step() {
  echo "[$(date +%H:%M:%S)] $*"
}

step "(1/4) 构建 wasm32-wasip2 调度器组件"
(
  cd "$SCHEDULER_DIR"
  cargo build --target wasm32-wasip2 --lib >/dev/null
)

if [[ ! -f "$COMPONENT_PATH" ]]; then
  echo "未找到 scheduler.wasm: $COMPONENT_PATH" >&2
  exit 1
fi

step "(2/4) 构建 demo http_server"
(
  cd "$SCHEDULER_DIR"
  cargo build --bin http_server >/dev/null
)

if [[ ! -x "$HTTP_SERVER_BIN" ]]; then
  echo "未找到 http_server 可执行文件: $HTTP_SERVER_BIN" >&2
  exit 1
fi

step "(3/4) 启动 http_server (日志: $SERVER_LOG)"
# 尽量清理已有 http_server（避免端口占用）
if pgrep -f "scheduler/target/.*/http_server" >/dev/null 2>&1; then
  pkill -f "scheduler/target/.*/http_server" || true
  sleep 0.2
fi

"$HTTP_SERVER_BIN" >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID >/dev/null 2>&1 || true' EXIT

SERVER_READY=0
for _ in {1..20}; do
  if grep -q "HTTP test server listening" "$SERVER_LOG" 2>/dev/null; then
    SERVER_READY=1
    break
  fi
  sleep 0.3
  if ! kill -0 $SERVER_PID >/dev/null 2>&1; then
    echo "http_server 进程异常退出，日志如下：" >&2
    cat "$SERVER_LOG" >&2
    exit 1
  fi
  echo "等待 http_server 就绪..."
  sleep 0.2
done

if [[ $SERVER_READY -ne 1 ]]; then
  echo "http_server 未在预期时间内就绪，日志：" >&2
  cat "$SERVER_LOG" >&2
  exit 1
fi

SERVER_LINE=$(grep 'HTTP test server listening' "$SERVER_LOG" | tail -n 1)
echo "http_server 已就绪：$SERVER_LINE"

step "(4/4) 运行顶层 Runner（日志: $RUN_LOG)"
(
  cd "$REPO_ROOT"
  SCHEDULER_COMPONENT="$COMPONENT_PATH" cargo run -- plugins/scheduler/res/simple_scenario.yaml |
    tee "$RUN_LOG"
)

if grep -q "Total actions executed: 1" "$RUN_LOG"; then
  step "验证通过：WASI simple_scenario 成功执行"
else
  echo "⚠️ 运行完成但未检测到成功指标，请检查 $RUN_LOG" >&2
  exit 1
fi

step "日志位置："
echo "  http_server: $SERVER_LOG"
echo "  runner 输出: $RUN_LOG"
