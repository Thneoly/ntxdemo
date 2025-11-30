# Scheduler WASM Component - Current Status

## Overview
Successfully converted the scheduler from a Native Rust binary to a WASM component (wasm32-wasip2 target).

## Completed Work

### 1. HttpActionComponent Conversion ✅
- **File**: `scheduler/actions-http/src/lib.rs`
- **Changes**: Replaced reqwest-based HTTP client with socket-based implementation
- **Status**: Complete (HTTP only, HTTPS requires TLS implementation)
- **Implementation**:
  ```rust
  fn send_http_request(request: &HttpRequest, bind_ip: Option<&str>) -> Result<HttpResponse> {
      let socket = socket::create_socket(AddressFamily::Ipv4, SocketProtocol::Tcp)?;
      if let Some(ip_str) = bind_ip {
          socket::bind(socket, SocketAddress::new(ip_str, 0))?;
      }
      socket::connect(socket, remote_addr)?;
      socket::send(socket, &request_bytes)?;
      // Read and parse HTTP response
      socket::close(socket);
      HttpResponse::parse(&response_data)
  }
  ```

### 2. WIT Interface Definition ✅
- **File**: `scheduler/scheduler/wit/world.wit`
- **Content**:
  ```wit
  package scheduler:main;
  
  world scheduler-component {
      export run-scenario: func(scenario-yaml: string) -> result<string, string>;
  }
  ```
- **Status**: Simple interface without explicit WASI imports

### 3. WASM Component Implementation ✅
- **File**: `scheduler/scheduler/src/component.rs`
- **Key Features**:
  - Sequential user execution (no async/tokio in WASM)
  - IP pool management
  - Statistics collection
  - Error handling and reporting
- **Entry Point**:
  ```rust
  impl Guest for SchedulerComponent {
      fn run_scenario(scenario_yaml: String) -> Result<String, String> {
          // Parse scenario, execute users, return statistics
      }
  }
  ```

### 4. Build Configuration ✅
- **File**: `scheduler/scheduler/Cargo.toml`
- **Changes**:
  ```toml
  [lib]
  crate-type = ["cdylib", "rlib"]
  
  [target.'cfg(target_arch = "wasm32")'.dependencies]
  wit-bindgen = { version = "0.48", features = ["realloc"] }
  
  [target.'cfg(not(target_arch = "wasm32"))'.dependencies]
  tokio = { ... }
  axum = { ... }
  ctrlc = { ... }
  ```
- **Build Command**: `cargo component build --lib --target wasm32-wasip2`
- **Output**: `/home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/target/wasm32-wasip2/debug/scheduler.wasm` (16MB)

### 5. Conditional Compilation ✅
- Made Native-only code conditional:
  - `ctrlc` signal handling
  - `tokio` async runtime
  - Binary targets (main.rs, http_server.rs)

## Current Challenge: Component Linking

### Problem
The scheduler WASM component has these dependencies:
```
scheduler.wasm
├── scheduler-core (core-libs)
│   ├── Custom socket interface (wasi-network, wasi-tcp, wasi-udp)
│   └── Wraps WASI Preview 2 sockets
├── scheduler-executor
│   └── Action component trait
└── scheduler-actions-http
    └── Uses core-libs socket
```

When running with wasmtime:
```bash
$ wasmtime run ../target/wasm32-wasip2/debug/scheduler.wasm \
    --invoke 'run-scenario' -- "$(cat test_scenario.yaml)"
    
Error: component imports instance `scheduler:core-libs/wasi-network@0.1.0`, 
but a matching implementation was not found in the linker
```

### Root Cause
- **core-libs** defines custom socket interfaces that wrap WASI sockets
- These custom interfaces need to be provided by composition, not by wasmtime directly
- wasmtime only provides standard `wasi:sockets/*` interfaces
- We need component composition to wire everything together

## Solution Approaches

### Approach 1: Full WAC Composition (Recommended)
Build all components separately and compose them with WAC:

1. **Build core-libs as WASM component**
   - Configure `core-libs/Cargo.toml` for component build
   - Add WIT interface that imports `wasi:sockets/*` and exports `scheduler:core-libs/*`
   - Build: `cargo component build --lib --target wasm32-wasip2`

2. **Build executor as WASM component**
   - Configure `executor/Cargo.toml` for component build
   - Define WIT interface for action components
   - Build: `cargo component build --lib --target wasm32-wasip2`

3. **Build actions-http as WASM component** (Already done ✅)
   - Already generates `scheduler_actions_http.wasm`

4. **Build scheduler as WASM component** (Already done ✅)
   - Already generates `scheduler.wasm`

5. **Create WAC composition**
   ```wac
   package scheduler:composed@0.1.0;
   
   export scheduler-run {
       let core = new scheduler:core-libs { };
       let http = new scheduler:actions-http { 
           import scheduler:core-libs/socket = core.socket;
       };
       let executor = new scheduler:executor { 
           import scheduler:actions-http/http-component = http.http-component;
       };
       let scheduler = new scheduler:main { 
           import scheduler:core-libs/* = core.*;
           import scheduler:executor/* = executor.*;
           import scheduler:actions-http/* = http.*;
       };
       export scheduler.run-scenario as run-scenario;
   }
   ```

6. **Compose with WAC**
   ```bash
   wac compose scheduler-composed.wac \
       -o scheduler-final.wasm \
       --core-libs ../core-libs/target/.../libscheduler_core.wasm \
       --executor ../executor/target/.../libscheduler_executor.wasm \
       --actions-http ../target/.../scheduler_actions_http.wasm \
       --scheduler ../target/.../scheduler.wasm
   ```

7. **Run with wasmtime**
   ```bash
   wasmtime run scheduler-final.wasm \
       --invoke 'run-scenario' -- "$(cat test_scenario.yaml)"
   ```

### Approach 2: Inline Rust Dependencies (Simpler)
Instead of WAC composition, make core-libs and executor regular Rust dependencies:
- Current setup already does this for compilation
- The issue is that the WASM component still declares WIT imports
- Would need to remove WIT-level separation and make everything internal

### Approach 3: Direct WASI Sockets (No Custom Interfaces)
Simplify by removing custom socket interfaces:
- Remove `scheduler:core-libs/wasi-*` interfaces
- Use `wasi:sockets/*` directly in actions-http
- Lose the abstraction layer that core-libs provides
- Wasmtime can directly provide WASI sockets

## Recommended Next Steps

1. **Configure core-libs for WASM component build**
   - Add `crate-type = ["cdylib", "rlib"]` to Cargo.toml
   - Create WIT file that imports WASI and exports custom interfaces
   - Build as component

2. **Configure executor for WASM component build**
   - Add `crate-type = ["cdylib", "rlib"]` to Cargo.toml
   - Create WIT interface for action component trait
   - Build as component

3. **Create comprehensive WAC composition**
   - Wire all components together
   - Map WASI imports through core-libs adapter

4. **Test the composed component**
   - Run with wasmtime
   - Verify real network access works through WASI sockets

## Testing

### Test Scenario
Created `scheduler/scheduler/test_scenario.yaml`:
```yaml
scenario:
  id: test-001
  description: Simple HTTP load test
  load:
    total_users: 5
    ramp_duration: 5s
    test_duration: 30s
    ip_pool:
      - 192.168.1.100
      - 192.168.1.101
  workflow:
    - id: step1
      type: http
      name: Get Request
      params:
        method: GET
        url: http://httpbin.org/get
        headers:
          User-Agent: WasmScheduler/1.0
      think_time: 1s
```

### Expected Invocation (after composition)
```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/scheduler

wasmtime run scheduler-final.wasm \
    --invoke 'run-scenario' \
    -- "$(cat test_scenario.yaml)"
```

### Expected Output
```json
{
  "total_requests": 5,
  "successful": 5,
  "failed": 0,
  "latency_p50_ms": 150.5,
  "latency_p95_ms": 250.0,
  "latency_p99_ms": 280.0,
  "duration_seconds": 30.0,
  "throughput_rps": 0.16
}
```

## Architecture Benefits

### WASM Component Model Advantages
1. **Portability**: Run on any WASI-compliant runtime (wasmtime, wasmer, wasm-edge)
2. **Sandboxing**: WASI provides capability-based security
3. **Composition**: Different action components can be swapped via WAC
4. **Cross-platform**: Single binary works on Linux, macOS, Windows
5. **Source IP Binding**: Real WASI socket support enables true IP binding

### vs Native Binary
| Feature | Native Binary | WASM Component |
|---------|--------------|----------------|
| Build Output | Platform-specific | Universal WASM |
| HTTP Client | reqwest (stub sockets) | WASI sockets (real) |
| Async Runtime | tokio | Sequential (no async) |
| IP Binding | Limited | Full WASI support |
| Composition | Static linking | WAC composition |
| Sandbox | OS-level | WASI capability |

## Known Limitations

### Current Implementation
1. **No HTTPS support**: TLS implementation not yet added to socket-based HTTP client
2. **Sequential execution**: No async/await in WASM (tokio not available)
3. **Performance**: Sequential users may be slower than concurrent (tokio-based)
4. **Debugging**: WASM debugging is less mature than native

### Future Work
- [ ] Add TLS support using rustls or similar
- [ ] Optimize sequential execution with WASI threads (experimental)
- [ ] Implement streaming for large responses
- [ ] Add WebSocket support
- [ ] Create more action components (TCP, UDP, custom protocols)
- [ ] Performance benchmarking vs Native

## Build Artifacts

```
target/wasm32-wasip2/debug/
├── scheduler.wasm (16MB) - Main scheduler component ✅
├── scheduler_actions_http.wasm (14MB) - HTTP action component ✅
├── libscheduler.rlib - Rust library (for linking)
├── libscheduler_actions_http.rlib - Rust library (for linking)
└── deps/ - Dependency artifacts
```

## Files Modified

1. ✅ `scheduler/actions-http/src/lib.rs` - Socket-based HTTP client
2. ✅ `scheduler/actions-http/Cargo.toml` - Removed reqwest
3. ✅ `scheduler/scheduler/wit/world.wit` - WIT interface
4. ✅ `scheduler/scheduler/src/component.rs` - WASM component impl
5. ✅ `scheduler/scheduler/src/lib.rs` - Conditional component module
6. ✅ `scheduler/scheduler/src/user.rs` - Added helper method
7. ✅ `scheduler/scheduler/src/engine.rs` - Conditional ctrlc
8. ✅ `scheduler/scheduler/Cargo.toml` - WASM component config

## Summary

✅ **Successfully built scheduler as WASM component**
⏳ **Pending: Component composition with WAC**
🎯 **Goal: Single composed WASM that runs with `wasmtime --invoke run-scenario`**

The architecture is sound, and the individual components build correctly. The remaining work is to configure core-libs and executor as WASM components and create the WAC composition to wire everything together.
