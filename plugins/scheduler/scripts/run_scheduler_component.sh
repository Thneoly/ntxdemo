#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

SCENARIO_FILE=${1:-res/http_scenario.yaml}
COMPONENT_FILE=${2:-wac/scheduler-composed.wasm}

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
