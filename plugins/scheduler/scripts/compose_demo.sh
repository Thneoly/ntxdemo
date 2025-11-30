#!/usr/bin/env bash
set -e

cd "$(dirname "$0")"

echo "=========================================="
echo "Simplified Component Composition Demo"
echo "=========================================="
echo ""
echo "This demo shows how to compose scheduler components."
echo "Currently only core-libs is fully functional as a component."
echo ""

# Build core-libs component
echo "Step 1/2: Building core-libs component..."
cd core-libs && ./build.sh
cd ..
echo ""

echo "Step 2/2: Component information..."
echo ""

if command -v wasm-tools &> /dev/null; then
    echo "==== Core-libs Component WIT Interface ===="
    wasm-tools component wit target/wasm32-wasip2/release/scheduler_core.wasm
    echo ""
    
    echo "==== Component Validation ===="
    if wasm-tools validate target/wasm32-wasip2/release/scheduler_core.wasm; then
        echo "✓ Component is valid!"
    fi
else
    echo "⚠ wasm-tools not installed"
    echo "Install with: cargo install wasm-tools"
fi

echo ""
echo "=========================================="
echo "✓ Demo complete!"
echo "=========================================="
echo ""
echo "Component output:"
echo "  target/wasm32-wasip2/release/scheduler_core.wasm"
echo ""
echo "To use with wasmtime:"
echo "  wasmtime run target/wasm32-wasip2/release/scheduler_core.wasm"
echo ""
echo "Next steps:"
echo "  - Fix executor and actions-http component implementations"
echo "  - Use 'wac compose' or 'wac plug' to link multiple components"
echo "  - See COMPONENTS.md for architecture details"
