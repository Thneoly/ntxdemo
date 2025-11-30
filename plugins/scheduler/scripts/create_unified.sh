#!/usr/bin/env bash
set -e

# Get the project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

echo "=========================================="
echo "Creating Unified Scheduler Component with WAC"
echo "=========================================="
echo ""

# Check prerequisites
if ! command -v wac &> /dev/null; then
    echo "ERROR: wac is not installed."
    echo "Install it with: cargo install wac-cli"
    exit 1
fi

if ! command -v wasm-tools &> /dev/null; then
    echo "WARNING: wasm-tools not found (optional for validation)"
    echo "Install with: cargo install wasm-tools"
fi

# Step 1: Ensure core component is built
echo "Step 1/3: Building core-libs component..."
cd "$PROJECT_ROOT/core-libs" && ./build.sh
cd "$PROJECT_ROOT"
echo ""

# Step 2: Create output directory
mkdir -p composed/target
echo "Step 2/3: Creating unified component..."
echo ""

# For now, we'll use wasm-tools to simply copy and adapt the core component
# as a "unified" component since we only have one working component
if command -v wasm-tools &> /dev/null; then
    # Copy the core component as the base for unified component
    cp target/wasm32-wasip2/release/scheduler_core.wasm composed/target/unified_scheduler.wasm
    
    echo "✓ Created unified component (currently wraps scheduler-core)"
    echo ""
    
    # Step 3: Validate
    echo "Step 3/3: Validating unified component..."
    echo ""
    
    if wasm-tools validate composed/target/unified_scheduler.wasm; then
        echo "✓ Component is valid!"
    fi
    
    echo ""
    echo "Component interface:"
    wasm-tools component wit composed/target/unified_scheduler.wasm | head -30
    echo "... (output truncated)"
else
    echo "ERROR: wasm-tools is required"
    exit 1
fi

echo ""
echo "=========================================="
echo "✓ Unified component created!"
echo "=========================================="
echo ""
echo "Output: composed/target/unified_scheduler.wasm"
echo ""
echo "This unified component currently contains:"
echo "  - scheduler:core-libs/types@0.1.0 (DSL data structures)"
echo "  - scheduler:core-libs/parser@0.1.0 (parse/validate functions)"
echo ""
echo "To use:"
echo "  wasm-tools component wit composed/target/unified_scheduler.wasm"
echo "  wasmtime run composed/target/unified_scheduler.wasm"
echo ""
echo "Future: When executor and actions-http are ready, this script"
echo "will use 'wac plug' to compose all three components."
