# Scheduler Components Architecture

This document describes the WebAssembly Component Model architecture for the scheduler plugin.

## Overview

The scheduler has been split into four independent crates that can each be compiled as WebAssembly components:

1. **scheduler-core**: Core data structures (DSL, WBS, state machine) ✅ **Fully functional**
2. **scheduler-executor**: Runtime abstractions (ActionComponent interface, event model) 🚧 **In progress**
3. **scheduler-actions-http**: HTTP action implementation 🚧 **In progress**
4. **scheduler**: Main binary and priority-loop engine

## Quick Start

### Build and validate a working component

```bash
cd plugins/scheduler
./scripts/compose_demo.sh
```

This builds the `scheduler-core` component and validates it. Output:
- `target/wasm32-wasip2/release/scheduler_core.wasm`

### Component Composition Status

**Current Status:**
- ✅ **core-libs**: Fully functional as a standalone component
  - Exports DSL parsing (`parse-scenario`, `validate-scenario`)
  - Exports all type definitions (`Scenario`, `ActionDef`, etc.)
  - Validated and ready for use

- 🚧 **executor**: Needs trait implementation fixes
  - Multiple `Guest` traits require proper implementation
  - Resource types (ActionContext) need lifetime management

- 🚧 **actions-http**: Depends on executor completion
  - HTTP component logic is ready
  - Waiting for executor interface stabilization

**Planned Composition:**
Once all components are ready, use `wac` to compose them:

```bash
./compose.sh  # Will compose all three when ready
```

## Component Interface Design

### Core Libs (`scheduler:core-libs@0.1.0`)

**Purpose**: Parse and validate workflow scenarios

**Exports**:
- `types` interface: Scenario, ActionDef, ResourceDef, WorkflowNode, etc.
- `parser` interface:
  - `parse-scenario(yaml: string) -> result<scenario, string>`
  - `validate-scenario(scenario) -> result<_, string>`

**Dependencies**: None (pure data and parsing)

### Executor (`scheduler:executor@0.1.0`)

**Purpose**: Define action execution contracts and event model

**Exports**:
- `types` interface: ActionOutcome, WbsTask, ActionStatus, etc.
- `context` interface: ActionContext resource with mutation helpers
- `component-api` interface:
  - `execute-action(action, context) -> result<outcome, string>`

**Dependencies**: scheduler:core-libs (for action definitions)

### Actions HTTP (`scheduler:actions-http@0.1.0`)

**Purpose**: Implement HTTP-based action component

**Exports**:
- `types` interface: ActionDef, ActionOutcome (re-export for convenience)
- `http-component` interface:
  - `init-component() -> result<_, string>`
  - `do-http-action(action) -> result<outcome, string>`
  - `release-component() -> result<_, string>`

**Dependencies**: 
- scheduler:executor (for action API contracts)
- wasi:http (for HTTP requests in wasm runtime)

## Building Components

Each crate has a `build.sh` script:

```bash
#!/usr/bin/env bash
set -e
cd "$(dirname "$0")"
cargo component build --target wasm32-wasip2 --release
```

Or use the master build script:

```bash
cd plugins/scheduler
./scripts/build_all_components.sh
```

## Using the Core Component

The `scheduler-core` component can be used immediately:

### Inspect the interface

```bash
wasm-tools component wit target/wasm32-wasip2/release/scheduler_core.wasm
```

### Use with wasmtime (example)

```bash
# Assuming you have a wasmtime-compatible host
wasmtime run target/wasm32-wasip2/release/scheduler_core.wasm
```

### Compose with other components

```wac
// Example WAC composition (future)
let core = new scheduler:core-libs("./scheduler_core.wasm");
let my-host = new my:host { ... };

// Connect core's parser to your host
my-host.parse-yaml -> core.parse-scenario;
```

## General Component Usage Patterns

Once all components are ready, they can be:
2. **Loaded in wasmtime**: Run individually or as composed graphs
3. **Embedded in native host**: The scheduler binary can load components dynamically

Example composition (pseudo-WAC):

```wac
let core = new scheduler:core-libs { ... }
let executor = new scheduler:executor { ... }
let http-action = new scheduler:actions-http { ... }

export core.*
export executor.*
export http-action.*
```

## Development Workflow

### Pure Rust (no wasm)

```bash
cargo build
cargo test
cargo run
```

Continues to work as before. Component code is conditionally compiled only for `target_arch = "wasm32"`.

### Wasm Component

```bash
./scripts/build_all_components.sh
wasmtime run target/wasm32-wasip2/release/scheduler_core.wasm
```

### Mixed (native + wasm)

The scheduler binary can dynamically load action components compiled to wasm, allowing third-party plugins without recompilation.

## Limitations & TODOs

**Completed:**
- [x] WIT interface definitions for all three crates
- [x] Component build infrastructure (Cargo.toml, build scripts)
- [x] Full implementation of scheduler-core component
- [x] Component validation and inspection tooling

**In Progress:**
- [ ] Multiple Guest trait implementations for executor
- [ ] Resource type lifetime management (ActionContext)
- [ ] Actions-http component implementation
- [ ] WASI HTTP integration for actions-http

**Future Work:**
- [ ] Full type conversion logic (currently simplified stubs)
- [ ] Multi-component composition examples with wac
- [ ] Dynamic component loading in scheduler binary
- [ ] Error propagation across component boundaries
- [ ] Performance benchmarks for component vs native calls

## References

- [WebAssembly Component Model](https://github.com/WebAssembly/component-model)
- [cargo-component docs](https://github.com/bytecodealliance/cargo-component)
- [wit-bindgen](https://github.com/bytecodealliance/wit-bindgen)
- [wac (WebAssembly Composition tool)](https://github.com/bytecodealliance/wac)
