# Using Scheduler Components

## Current Architecture

All four scheduler components have been successfully built as wasm32-wasip2 components:

1. `scheduler_core.wasm` - Core libraries (types, parser, socket)
2. `scheduler_executor.wasm` - Action executor
3. `scheduler_actions_http.wasm` - HTTP actions
4. `scheduler.wasm` - Main scheduler with run-scenario function

**Important:** Currently, all dependencies are handled via Rust's Cargo system, which means:
- `scheduler.wasm` already includes all functionality from the other three components
- The components are statically linked, not dynamically composed via WIT
- You can use `scheduler.wasm` directly without WAC composition

## Quick Start: Using scheduler.wasm

### Option 1: Use with Wasmtime CLI

```bash
# Navigate to the scheduler directory
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler

# Run with a scenario file
wasmtime run target/wasm32-wasip2/debug/scheduler.wasm -- scenario.yaml
```

### Option 2: Use with Wasmtime Engine API (Rust)

```rust
use anyhow::Result;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::WasiCtxBuilder;

// Define the component interface
wasmtime::component::bindgen!({
    path: "plugins/scheduler/wit/world.wit",
    world: "scheduler",
    async: false,
});

fn main() -> Result<()> {
    // Configure engine for components
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;

    // Load the scheduler component
    let component = Component::from_file(
        &engine,
        "plugins/scheduler/target/wasm32-wasip2/debug/scheduler.wasm",
    )?;

    // Create linker and add WASI
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker)?;

    // Create store with WASI context
    let wasi_ctx = WasiCtxBuilder::new()
        .inherit_stdio()
        .inherit_env()
        .build();
    let mut store = Store::new(&engine, wasi_ctx);

    // Instantiate and call run-scenario
    let (scheduler, _) = Scheduler::instantiate(&mut store, &component, &linker)?;
    
    let scenario_yaml = std::fs::read_to_string("scenario.yaml")?;
    let result = scheduler.call_run_scenario(&mut store, &scenario_yaml)?;
    
    match result {
        Ok(output) => println!("Success: {}", output),
        Err(e) => eprintln!("Error: {}", e),
    }

    Ok(())
}
```

## Component Details

### scheduler.wasm Interface

**Exported Function:**
```wit
run-scenario: func(scenario-yaml: string) -> result<string, string>
```

**Parameters:**
- `scenario-yaml: string` - YAML scenario definition

**Returns:**
- `Ok(string)` - Success message with execution details
- `Err(string)` - Error message

**Example Scenario:**
```yaml
name: test-scenario
ip_pools:
  - name: default
    ips:
      - 192.168.1.100
      - 192.168.1.101

actions:
  - id: http-get
    call: http.request
    with:
      method: GET
      url: https://httpbin.org/get
      headers:
        User-Agent: Scheduler/1.0

tasks:
  - id: start
    next:
      - task: t1
  - id: t1
    action: http-get
    next:
      - task: end
  - id: end
```

## Future: WAC Composition

Currently not needed since dependencies are statically linked, but for future reference:

### When to Use WAC Composition

Use WAC composition when you want to:
1. **Replace Components at Runtime** - Swap different implementations without rebuilding
2. **Share Components** - Multiple consumers using the same component instance
3. **Dynamic Linking** - Reduce binary size by sharing common code
4. **Plugin Architecture** - Load different action components dynamically

### Example WAC Composition

```wac
package scheduler:final@0.1.0;

// Instantiate components
let core = new scheduler:core-libs { };
let scheduler = new scheduler:main { };

// Wire WASI imports
import wasi:io/poll = wasi:io/poll@0.2.6;
import wasi:cli/stdout = wasi:cli/stdout@0.2.6;
// ... other WASI imports

// Export main function
export scheduler.run-scenario;
```

### Compose with WAC

```bash
wac compose scheduler-composition.wac \
    -d scheduler:core-libs=target/wasm32-wasip2/debug/scheduler_core.wasm \
    -d scheduler:main=target/wasm32-wasip2/debug/scheduler.wasm \
    -o final-scheduler.wasm
```

## Testing

### 1. Build All Components

```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler

# Build core-libs
cd core-libs && cargo component build --lib --target wasm32-wasip2 && cd ..

# Build executor
cd executor && cargo component build --lib --target wasm32-wasip2 && cd ..

# Build actions-http
cd actions-http && cargo component build --lib --target wasm32-wasip2 && cd ..

# Build scheduler
cargo component build --lib --target wasm32-wasip2
```

### 2. Verify Component Interfaces

```bash
# Check all components only import standard WASI
for f in target/wasm32-wasip2/debug/scheduler*.wasm; do
  echo "=== $(basename $f) ==="
  wasm-tools component wit "$f" 2>&1 | grep -E "^  (import|export)"
  echo ""
done
```

Expected output:
- All components should only import `wasi:io/*`, `wasi:cli/*`, etc.
- No imports of custom `scheduler:core-libs/wasi-*` interfaces
- Each component exports its specific interfaces

### 3. Test with Example Scenario

```bash
# Create a test scenario
cat > test-scenario.yaml <<EOF
name: simple-test
ip_pools:
  - name: default
    ips: [127.0.0.1]
actions:
  - id: test-action
    call: http.request
    with:
      method: GET
      url: http://httpbin.org/get
tasks:
  - id: start
    next: [{task: t1}]
  - id: t1
    action: test-action
    next: [{task: end}]
  - id: end
EOF

# Run with wasmtime (when main.rs is updated to use component model)
wasmtime run target/wasm32-wasip2/debug/scheduler.wasm -- test-scenario.yaml
```

## Troubleshooting

### "unknown import" errors
- Check that all components only import standard WASI interfaces
- Run `wasm-tools component wit <component.wasm>` to inspect imports

### Component won't instantiate
- Ensure wasmtime version supports WASI Preview 2 (0.38+)
- Check that Config enables component model: `config.wasm_component_model(true)`
- Verify WASI context is properly configured in Store

### Socket operations fail
- Current implementation uses stubs that return errors
- Real socket implementation requires:
  - WASI Preview 2 socket support in runtime
  - Or adapter that maps to host sockets
  - Or alternative implementation via WASI HTTP

## Next Steps

1. **Update main.rs** - Modify to use wasmtime component API instead of direct execution
2. **Implement Real Sockets** - Replace stub implementation with actual WASI socket calls
3. **Add More Actions** - Create additional action components (TCP, UDP, etc.)
4. **Performance Testing** - Benchmark component vs native performance
5. **WAC Composition** - When needed for dynamic component replacement

## Architecture Documentation

See also:
- `COMPONENT_STATUS.md` - Current build status and interface details
- `scheduler-composition.wac` - Example WAC composition file
- `ARCHITECTURE.md` - Overall system architecture
- `WHY_REQWEST.md` - Background on socket vs reqwest decision
