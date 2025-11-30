#!/usr/bin/env bash
# Example script to test actions-http component

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "=========================================="
echo "Actions-HTTP Component Test"
echo "=========================================="
echo

# Check if component exists
COMPONENT="../target/wasm32-wasip2/release/scheduler_actions_http.wasm"
if [ ! -f "$COMPONENT" ]; then
    echo "❌ Component not found: $COMPONENT"
    echo "Building component..."
    cargo component build --target wasm32-wasip2 --release
fi

echo "✓ Component found: $COMPONENT"
echo

# Show component info
echo "Component Info:"
echo "---------------"
ls -lh "$COMPONENT"
echo

# Inspect WIT interfaces
echo "Exported Interfaces:"
echo "-------------------"
wasm-tools component wit "$COMPONENT" 2>/dev/null | grep "export scheduler" || true
echo

echo "Imported Interfaces:"
echo "-------------------"
wasm-tools component wit "$COMPONENT" 2>/dev/null | grep "import scheduler" || true
echo

# Test action example (JSON format for component)
echo "Example Action DSL:"
echo "-------------------"
cat <<'EOF'
{
  "id": "test-http-get",
  "call": "GET",
  "with-params": "{\"url\":\"http://192.168.1.100:8080/api/status\",\"headers\":{\"User-Agent\":\"Test\"}}",
  "exports": []
}
EOF
echo

echo "=========================================="
echo "✅ Component is ready to use!"
echo "=========================================="
echo
echo "Next steps:"
echo "1. Load this component in your executor/scheduler"
echo "2. Pass ActionDef to do-http-action function"
echo "3. Receive ActionOutcome with HTTP response"
echo
echo "See IMPLEMENTATION_SUMMARY.md for usage examples"
