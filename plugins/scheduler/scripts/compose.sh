#!/usr/bin/env bash
set -e

cd "$(dirname "$0")"

echo "=========================================="
echo "Building and Composing Scheduler Components"
echo "=========================================="
echo ""

# Step 1: Build all individual components
echo "Step 1/3: Building individual components..."
./build_all_components.sh
echo ""

# Step 2: Use wac to compose them
echo "Step 2/3: Composing components with wac..."
echo ""

# Check if wac is installed
if ! command -v wac &> /dev/null; then
    echo "ERROR: wac is not installed."
    echo "Install it with: cargo install wac-cli"
    exit 1
fi

# Create output directory
mkdir -p composed/target

# Compose the components
echo "Running: wac plug ..."
wac plug \
    --plug target/wasm32-wasip2/release/scheduler_core.wasm \
    --plug target/wasm32-wasip2/release/scheduler_executor.wasm \
    --plug target/wasm32-wasip2/release/scheduler_actions_http.wasm \
    -o composed/target/unified_scheduler.wasm

echo ""
echo "Step 3/3: Validating composed component..."

# Validate the composed component
if command -v wasm-tools &> /dev/null; then
    echo "Component info:"
    wasm-tools component wit composed/target/unified_scheduler.wasm
    echo ""
    echo "Component validation:"
    wasm-tools validate composed/target/unified_scheduler.wasm && echo "✓ Component is valid"
else
    echo "⚠ wasm-tools not found, skipping validation"
    echo "Install with: cargo install wasm-tools"
fi

echo ""
echo "=========================================="
echo "✓ Composition complete!"
echo "=========================================="
echo ""
echo "Output: composed/target/unified_scheduler.wasm"
echo ""
echo "To inspect the component:"
echo "  wasm-tools component wit composed/target/unified_scheduler.wasm"
echo ""
echo "To run with wasmtime:"
echo "  wasmtime run composed/target/unified_scheduler.wasm"
