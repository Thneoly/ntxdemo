#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET="wasm32-wasip2"

echo "=========================================="
echo "Building scheduler wasm components"
echo "=========================================="
echo ""

build_component() {
	local crate="$1"
	local label="$2"
	echo "→ Building ${label} (${crate})"
	(
		cd "$PROJECT_ROOT/$crate"
		cargo build --target "$TARGET"
	)
	echo ""
}

build_component core-libs "core-libs"
build_component actions-executor "actions-executor"
build_component eventbus "eventbus"
build_component scheduler "scheduler host"

echo "Artifacts (debug profile):"
echo "  - core-libs/target/${TARGET}/debug/scheduler_core.wasm"
echo "  - actions-executor/target/${TARGET}/debug/scheduler_actions_executor.wasm"
echo "  - eventbus/target/${TARGET}/debug/eventbus.wasm"
echo "  - scheduler/target/${TARGET}/debug/scheduler.wasm"
echo ""
echo "Tip: set CARGO_BUILD_TARGET_DIR or use \`cargo build --release\` if you need optimized artifacts."
