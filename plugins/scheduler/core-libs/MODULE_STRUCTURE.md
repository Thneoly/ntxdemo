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
├── ip/                    # IP pool management
│   └── mod.rs
├── socket/                # Socket API with WASI implementation
│   ├── mod.rs             # Public socket interface
│   ├── api.rs             # High-level Socket API with IP binding
│   └── wasi_impl.rs       # WASI socket implementation
├── state_machine/         # State machine logic
│   └── mod.rs
├── wbs/                   # Work Breakdown Structure
│   └── mod.rs
└── workbook/              # Workbook management
    └── mod.rs
```

## Module Responsibilities

### ip/
- **Purpose**: IP address pool management system
- **Features**:
  - IP range definition (CIDR notation support)
  - IP allocation and release
  - Binding IPs to resources via subinstance/subid/subtype
  - Support for multiple resource types (MAC, VM, Container, Pod, Custom)
  - Reserved IP management
  - Pool statistics and monitoring
- **API**: IpPool, IpRange, IpBinding, ResourceType

### socket/
- **Purpose**: Cross-platform socket API with real WASI networking support and IP binding
- **Files**: 
  - `mod.rs`: Module definitions and raw socket functions
  - `api.rs`: High-level Socket struct with state management and IP pool integration
  - `wasi_impl.rs`: Real WASI socket implementation for WASM targets
- **Features**:
  - **High-level Socket API**: Object-oriented socket programming (socket/bind/listen/connect/send/recv)
  - **IP Pool Integration**: Bind sockets to IPs allocated from IP pools
  - **State Management**: Track socket state (Created/Bound/Listening/Connected/Closed)
  - Conditional compilation for WASM vs native targets
  - WASI Preview 2 socket integration (TCP/UDP)
  - Two-phase locking pattern for resource management
  - Stub implementations for testing on native platforms
  - **Socket struct**: High-level API with methods like `bind_to_ip()`, `connect()`, `send()`, `recv()`
  - **Foundation for HTTP component**: Provides base layer for building HTTP client/server

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

### Socket + IP Pool Integration
```rust
// 1. Allocate IP from pool
let ip = pool.allocate("tenant", "resource", ResourceType::Vm("vm1".into()))?;

// 2. Create and bind socket to the IP
let mut sock = Socket::tcp_v4()?;
sock.bind_to_ip(ip, 8080)?;

// 3. Use socket for network I/O
sock.listen(100)?;
let client = sock.accept()?;
```

This pattern enables precise control over which IP address a socket uses for sending/receiving packets.

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

## Usage Examples

### TCP Server with IP Pool
```rust
use scheduler_core::{IpPool, ResourceType, Socket};

let mut pool = IpPool::new("server-pool");
pool.add_cidr_range("192.168.1.0/24")?;

let ip = pool.allocate("services", "http-server", 
    ResourceType::Custom("tcp-server".into()))?;

let mut server = Socket::tcp_v4()?;
server.bind_to_ip(ip, 8080)?;
server.listen(128)?;

let mut client = server.accept()?;
let request = client.recv(4096)?;
client.send(b"HTTP/1.1 200 OK\r\n\r\nHello")?;
```

### Multi-Tenant Sockets
```rust
// Tenant A
let ip_a = pool.allocate("tenant-a", "web", ResourceType::Vm("vm1".into()))?;
let mut sock_a = Socket::tcp_v4()?;
sock_a.bind_to_ip(ip_a, 80)?;

// Tenant B
let ip_b = pool.allocate("tenant-b", "api", ResourceType::Container("c1".into()))?;
let mut sock_b = Socket::tcp_v4()?;
sock_b.bind_to_ip(ip_b, 8080)?;
```

## Migration Notes

### From Previous Structure
- `src/socket.rs` → `src/socket/mod.rs`
- `src/socket_wasi_impl.rs` → `src/socket/wasi_impl.rs` (now internal)
- **New**: `src/socket/api.rs` - High-level Socket API
- All single-file modules moved to `<module>/mod.rs` pattern
- Module paths updated: `crate::socket_wasi_impl::` → `wasi_impl::`
- Import paths changed: `super::Type` → `crate::socket::Type` (in wasi_impl)

### Breaking Changes
- `socket_wasi_impl` is no longer a public module (internal to `socket`)
- All external code should use `Socket` struct for high-level operations
- Raw socket functions still available for advanced use cases

### New API
- `Socket` struct - High-level socket programming interface
- `Socket::bind_to_ip(ip, port)` - Bind socket to IP from pool
- `Socket::bind_with_binding(binding, port)` - Bind using IpBinding
- State tracking: `is_bound()`, `is_connected()`, `is_listening()`

## Testing

All 26 unit tests pass on native targets:
- socket::api: 5 tests (Socket creation, bind, bind_to_ip, lifecycle)
- ip: 6 tests (range creation, CIDR parsing, allocation, release, resource lookup, stats)
- socket: 5 tests (TCP creation, connect, bind/listen, UDP creation, send/receive)
- dsl: 3 tests (parsing, validation)
- state_machine: 3 tests (transitions, sync, triggers)
- wbs: 3 tests (tree building, mutations, conditions)
- workbook: 1 test (resource indexes)

## Documentation

- **Socket + IP Integration**: `doc/SOCKET_IP_INTEGRATION.md` - Complete guide
- **Socket Quick Reference**: `doc/SOCKET_QUICK_REFERENCE.md` - API cheat sheet
- **IP Pool Usage**: `doc/IP_POOL_USAGE.md` - IP pool management guide
- **WASI Implementation**: `doc/WASI_SOCKET_IMPLEMENTATION.md`
- **Module Overview**: This document
- **Examples**: `examples/socket_with_ip_pool.rs` - 4 practical scenarios
