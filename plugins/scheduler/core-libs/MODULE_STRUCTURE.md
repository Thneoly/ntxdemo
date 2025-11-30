# Core-Libs Module Structure

## Overview

The core-libs project has been reorganized to use a directory-based module structure, where each major module has its own directory with a `mod.rs` file as the entry point.

## Directory Structure

```
src/
├── component.rs           # Component bindings (generated)
├── lib.rs                 # Crate root with module declarations
├── README.md              # Module documentation
├── dsl/                   # DSL parsing and validation
│   └── mod.rs
├── error/                 # Error types
│   └── mod.rs
├── socket/                # Socket API with WASI implementation
│   ├── mod.rs             # Public socket interface
│   └── wasi_impl.rs       # WASI socket implementation
├── state_machine/         # State machine logic
│   └── mod.rs
├── wbs/                   # Work Breakdown Structure
│   └── mod.rs
└── workbook/              # Workbook management
    └── mod.rs
```

## Module Responsibilities

### socket/
- **Purpose**: Cross-platform socket API with real WASI networking support
- **Files**: 
  - `mod.rs`: Public API (14 functions: create, connect, bind, listen, accept, send, receive, etc.)
  - `wasi_impl.rs`: Real WASI socket implementation for WASM targets
- **Features**:
  - Conditional compilation for WASM vs native targets
  - WASI Preview 2 socket integration (TCP/UDP)
  - Two-phase locking pattern for resource management
  - Stub implementations for testing on native platforms

### dsl/
- **Purpose**: Domain-specific language parsing and validation
- **API**: Parse workflow definitions, validate scenarios

### error/
- **Purpose**: Common error types used throughout the library
- **API**: CoreLibError enum with various error variants

### state_machine/
- **Purpose**: State machine construction and management
- **API**: Build and manipulate state machines from workflow definitions

### wbs/
- **Purpose**: Work Breakdown Structure generation and management
- **API**: Task tree construction, dependency tracking

### workbook/
- **Purpose**: Workbook and resource management
- **API**: Resource allocation, metric tracking

## Build Targets

### WASM Component (wasm32-wasip1)
```bash
cargo component build --release
```
- Output: `target/wasm32-wasip1/release/scheduler_core.wasm` (~474 KB)
- Includes real WASI socket implementation
- Imports: wasi-network, wasi-tcp, wasi-udp
- Exports: types, parser, socket

### Native (for testing)
```bash
cargo test
```
- Uses stub socket implementations
- All 15 tests pass
- No WASI dependencies required

## Key Design Patterns

### Two-Phase Locking (socket/wasi_impl.rs)
```rust
// Phase 1: Get &mut access to ensure network resource exists
ensure_network()?;

// Phase 2: Get & access to read the network resource
let network = network()?;
```

This pattern avoids Rust borrow checker conflicts when managing WASI resources.

### Conditional Compilation
```rust
#[cfg(target_arch = "wasm32")]
pub use wasm_impl::*;  // Real WASI implementation

#[cfg(not(target_arch = "wasm32"))]
// Stub implementations for testing
```

Allows the same API to work on both WASM and native platforms.

## Migration Notes

### From Previous Structure
- `src/socket.rs` → `src/socket/mod.rs`
- `src/socket_wasi_impl.rs` → `src/socket/wasi_impl.rs` (now internal)
- All single-file modules moved to `<module>/mod.rs` pattern
- Module paths updated: `crate::socket_wasi_impl::` → `wasi_impl::`
- Import paths changed: `super::Type` → `crate::socket::Type` (in wasi_impl)

### Breaking Changes
- `socket_wasi_impl` is no longer a public module (internal to `socket`)
- All external code should use `socket::` APIs instead

## Testing

All 15 unit tests pass on native targets:
- socket: 5 tests (TCP creation, connect, bind/listen, UDP creation, send/receive)
- dsl: 3 tests (parsing, validation)
- state_machine: 3 tests (transitions, sync, triggers)
- wbs: 3 tests (tree building, mutations, conditions)
- workbook: 1 test (resource indexes)

## Documentation

- **API Documentation**: See individual module `mod.rs` files
- **WASI Implementation**: `WASI_SOCKET_IMPLEMENTATION.md`
- **Module Overview**: This document
