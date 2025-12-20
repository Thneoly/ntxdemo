//! ntx-guestnet: a strict, non-blocking guest networking stack.
//!
//! Extracted from `plugins/scheduler` so it can be reused by other guest components.
//!
//! Layering rules:
//! - Host interface is packet primitives only (no sockets)
//! - Parsing/encoding is confined to Transport
//! - No implicit blocking; backpressure is explicit via WouldBlock

// This crate is intended to be built as a WASM component (wasm32-wasip2).
//
// We still allow *host builds* for unit tests and CI (so `cargo test` works).
// For stricter enforcement, we rely on build tooling/CI to compile for wasm32.

pub mod driver;
pub mod flow;
pub mod host_if;
pub mod packet_io;
pub mod socket_api;
pub mod transport;

pub use driver::*;
pub use flow::*;
pub use host_if::*;
pub use packet_io::*;
pub use socket_api::*;
pub use transport::*;
