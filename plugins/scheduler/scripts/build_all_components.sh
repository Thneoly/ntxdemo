#!/usr/bin/env bash
set -e

# Get the project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

echo "=========================================="
echo "Building all scheduler components"
echo "=========================================="
echo ""

echo "Step 1/3: Building core-libs..."
cd "$PROJECT_ROOT/core-libs" && ./build.sh && cd "$PROJECT_ROOT"
echo ""

echo "Step 2/3: Building executor..."
cd "$PROJECT_ROOT/executor" && ./build.sh && cd "$PROJECT_ROOT" || echo "⚠️  Executor build failed (expected)"
echo ""

echo "Step 3/3: Building actions-http..."
cd "$PROJECT_ROOT/actions-http" && ./build.sh && cd "$PROJECT_ROOT" || echo "⚠️  Actions-HTTP build failed (expected)"
echo ""

echo "=========================================="
echo "✓ Build process complete!"
echo "=========================================="
echo ""
echo "Component outputs:"
echo "  - core-libs/target/wasm32-wasip2/release/scheduler_core.wasm"
echo "  - executor/target/wasm32-wasip2/release/scheduler_executor.wasm (🚧)"
echo "  - actions-http/target/wasm32-wasip2/release/scheduler_actions_http.wasm (🚧)"
