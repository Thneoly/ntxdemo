//! ntx-guestnet: a strict, non-blocking guest networking stack.
//!
//! Extracted from `plugins/scheduler` so it can be reused by other guest components.
//!
//! Layering rules:
//! - Host interface is packet primitives only (no sockets)
//! - Parsing/encoding is confined to Transport
//! - No implicit blocking; backpressure is explicit via WouldBlock

pub mod driver;
pub mod flow;
pub mod host_if;
pub mod packet_io;
pub mod socket_api;
pub mod transport;

// Documentation-only module preserved from the original extraction.
mod guestnet;

pub use driver::*;
pub use flow::*;
pub use host_if::*;
pub use packet_io::*;
pub use socket_api::*;
pub use transport::*;

// Keep unit tests colocated with their modules.
#[cfg(test)]
mod packet_io_tests;

#[cfg(test)]
mod packet_io_injected_tests;

#[cfg(test)]
mod flow_tests;

#[cfg(test)]
mod transport_tests;

#[cfg(test)]
mod socket_api_tests;

#[cfg(test)]
mod driver_tests;

#[cfg(test)]
mod tx_tests;
