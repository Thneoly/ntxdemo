# Using the Unified Scheduler Component

This guide shows how to use the unified scheduler component created by WAC composition.

## Current Status

✅ **Available Now**: Unified component with scheduler-core functionality
- File: `composed/target/unified_scheduler.wasm` (430KB)
- Exports: DSL parsing and type definitions

## Quick Start

### Inspect the Component

```bash
# View the WIT interface
wasm-tools component wit composed/target/unified_scheduler.wasm

# Validate the component
wasm-tools validate composed/target/unified_scheduler.wasm
```

### Rebuild the Component

```bash
# Rebuild with current components
./scripts/create_unified.sh

# See the future full composition plan
./scripts/compose_full.sh
```

## Using with Wasmtime

```bash
# Run the component (requires wasmtime with component model support)
wasmtime run composed/target/unified_scheduler.wasm
```

## Integration Example

### From Rust

```rust
use wasmtime::component::*;
use wasmtime::{Engine, Store};

let engine = Engine::default();
let mut store = Store::new(&engine, ());

// Load the unified component
let component = Component::from_file(&engine, "composed/target/unified_scheduler.wasm")?;

// Instantiate and call
let linker = Linker::new(&engine);
let instance = linker.instantiate(&mut store, &component)?;

// Call parse-scenario function
let parse_fn = instance.get_typed_func::<(&str,), (Result<Scenario, String>,)>(&mut store, "parse-scenario")?;
let result = parse_fn.call(&mut store, ("version: 1.0\nname: test\n...",))?;
```

### From JavaScript (with jco)

```javascript
import { parseScenario, validateScenario } from './unified_scheduler.js';

// Parse a YAML scenario
const yaml = `
version: "1.0"
name: "my-workflow"
workflows:
  nodes:
    - id: "start"
      type: "action"
`;

const result = parseScenario(yaml);
if (result.tag === 'ok') {
    console.log('Parsed scenario:', result.val);
    
    // Validate it
    const validation = validateScenario(result.val);
    console.log('Validation:', validation);
}
```

## Future Full Composition

When all three components are ready, the unified component will include:

### Exports

- **scheduler:core-libs/types@0.1.0**: Type definitions
- **scheduler:core-libs/parser@0.1.0**: DSL parsing
- **scheduler:executor/component-api@0.1.0**: Action execution
- **scheduler:actions-http/http-component@0.1.0**: HTTP actions

### Composition Command

```bash
wac plug \
    --plug target/wasm32-wasip2/release/scheduler_core.wasm \
    --plug target/wasm32-wasip2/release/scheduler_executor.wasm \
    --plug target/wasm32-wasip2/release/scheduler_actions_http.wasm \
    composed/socket.wasm \
    -o composed/target/unified_scheduler.wasm
```

### Full API Example

```rust
// Parse a scenario
let scenario = parse_scenario(yaml_string)?;

// Create execution context
let ctx = ActionContext::new();

// Execute an action
let action = ActionDef { /* ... */ };
let outcome = execute_action(action, ctx)?;

// Or use HTTP-specific implementation
init_component()?;
let result = do_http_action(action)?;
release_component()?;
```

## Component Architecture

```
┌─────────────────────────────────────┐
│   Unified Scheduler Component       │
│   (unified_scheduler.wasm)          │
│                                     │
│  ┌───────────────┐                 │
│  │  Core-Libs    │  Types & Parser │
│  └───────┬───────┘                 │
│          │                          │
│  ┌───────▼───────┐                 │
│  │  Executor     │  Runtime API    │
│  └───────┬───────┘                 │
│          │                          │
│  ┌───────▼───────┐                 │
│  │ Actions-HTTP  │  HTTP Impl      │
│  └───────────────┘                 │
└─────────────────────────────────────┘
```

## Troubleshooting

**Q: Component fails to instantiate**
- Ensure wasmtime supports Component Model
- Check WASI imports are satisfied

**Q: Cannot find exported functions**
- Use `wasm-tools component wit` to inspect exports
- Verify the component interface matches expectations

**Q: Composition fails**
- Ensure all plug components are built
- Check WIT compatibility between components

## Next Steps

1. Fix executor Guest trait implementations
2. Complete actions-http component bindings  
3. Re-run `./scripts/create_unified.sh` to get full composition
4. Deploy unified component to production runtime
