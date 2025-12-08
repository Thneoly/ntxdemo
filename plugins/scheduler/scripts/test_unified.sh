#!/bin/bash
# Test the unified scheduler component

set -e

# Get the project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

echo "🧪 Testing Unified Scheduler Component"
echo "======================================"
echo ""

# Check if component exists
if [ ! -f "composed/target/unified_scheduler.wasm" ]; then
    echo "❌ Component not found. Building first..."
    ./scripts/create_unified.sh
fi

echo "📊 Component Information:"
echo "------------------------"
ls -lh composed/target/unified_scheduler.wasm

echo ""
echo "🔍 Component Interface:"
echo "----------------------"
wasm-tools component wit composed/target/unified_scheduler.wasm | head -50

echo ""
echo "✅ Component Validation:"
echo "-----------------------"
if wasm-tools validate composed/target/unified_scheduler.wasm; then
    echo "✅ Component is valid"
else
    echo "❌ Component validation failed"
    exit 1
fi

echo ""
echo "📦 Exported Interfaces:"
echo "----------------------"
wasm-tools component wit composed/target/unified_scheduler.wasm | grep -E "(interface|export|world)" | head -20

echo ""
echo "🎯 Status Summary:"
echo "-----------------"
echo "✅ Core-libs: Functional (types + parser)"
echo "🚧 Executor: Needs Guest trait implementations"
echo "🚧 Actions-Executor: Waiting for executor"

echo ""
echo "📖 See doc/USAGE.md for integration examples"
