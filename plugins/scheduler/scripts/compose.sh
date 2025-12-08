#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEPS_DIR="$PROJECT_ROOT/wac/deps/scheduler"
OUTPUT="$PROJECT_ROOT/wac/scheduler-composed.wasm"

cd "$PROJECT_ROOT"

echo "=========================================="
echo "Building + composing scheduler component"
echo "=========================================="
echo ""

./scripts/build_all_components.sh

mkdir -p "$DEPS_DIR"

cp target/wasm32-wasip2/debug/scheduler.wasm "$DEPS_DIR/main.wasm"
cp target/wasm32-wasip2/debug/scheduler_actions_http.wasm "$DEPS_DIR/action-http.wasm"
cp target/wasm32-wasip2/debug/scheduler_actions_http.wasm "$DEPS_DIR/actions-http.wasm"
cp target/wasm32-wasip2/debug/scheduler_core.wasm "$DEPS_DIR/core-libs.wasm"
cp target/wasm32-wasip2/debug/eventbus.wasm "$DEPS_DIR/event-bus.wasm"

echo "Running wac compose..."
wac compose wac/scheduler-composition.wac --deps-dir wac/deps -o "$OUTPUT"

echo ""
if command -v wasm-tools &> /dev/null; then
    echo "Component summary (trimmed):"
    wasm-tools component wit "$OUTPUT" | head -40
    echo ""
    wasm-tools validate "$OUTPUT" && echo "✓ Component is valid"
else
    echo "(tip) install wasm-tools for validation: cargo install wasm-tools"
fi

echo ""
echo "Output ready at $OUTPUT"
echo "Run with:"
echo "  wasmtime run --wasi tcp=y --wasi inherit-network=y --invoke=run-scenario wac/scheduler-composed.wasm"
