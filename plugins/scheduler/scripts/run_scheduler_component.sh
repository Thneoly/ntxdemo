#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

SCENARIO_FILE=${1:-res/http_scenario.yaml}
COMPONENT_FILE=${2:-wac/scheduler-composed.wasm}

pushd actions-executor
  cargo build --target wasm32-wasip2
popd

pushd eventbus
  cargo build --target wasm32-wasip2
popd

pushd core-libs
  cargo build --target wasm32-wasip2
popd

pushd scheduler
  cargo build --no-default-features --target wasm32-wasip2
popd

cp target/wasm32-wasip2/debug/scheduler.wasm wac/deps/scheduler/main.wasm
cp target/wasm32-wasip2/debug/scheduler_actions_executor.wasm wac/deps/scheduler/action-executor.wasm
cp target/wasm32-wasip2/debug/scheduler_actions_executor.wasm wac/deps/scheduler/actions-executor.wasm
cp target/wasm32-wasip2/debug/scheduler_core.wasm wac/deps/scheduler/core-libs.wasm
cp target/wasm32-wasip2/debug/eventbus.wasm wac/deps/scheduler/event-bus.wasm

wac compose wac/scheduler-composition.wac --deps-dir wac/deps -o wac/scheduler-composed.wasm

if [[ ! -f "$SCENARIO_FILE" ]]; then
  echo "Scenario file not found: $SCENARIO_FILE" >&2
  exit 1
fi

if [[ ! -f "$COMPONENT_FILE" ]]; then
  echo "Component file not found: $COMPONENT_FILE" >&2
  echo "Hint: run ./scripts/create_unified.sh or rebuild your WAC composition." >&2
  exit 1
fi

WAVE_CALL=$(python - "$SCENARIO_FILE" <<'PY'
import pathlib, sys

scenario_path = pathlib.Path(sys.argv[1])

text = scenario_path.read_text()
text = text.replace('\\', '\\\\')
text = text.replace('"""', '\\"""')
call = f'run-scenario("""\n{text}\n""")'
print(call)
PY
)

export WASMTIME_BACKTRACE_DETAILS="${WASMTIME_BACKTRACE_DETAILS:-1}"

set -x
wasmtime run \
  --wasi tcp=y \
  --wasi inherit-network=y \
  --invoke "$WAVE_CALL" \
  "$COMPONENT_FILE"
