pub mod audit_registry;
pub mod error;
pub mod event_bus;
pub mod kernel;
pub mod logger;
pub mod scheduler;
pub mod time;

/// Network stack crate re-export.
///
/// Many examples/tests reference `ntx::network::*` as a stable path.
pub use ntx_network as network;

// Keep a few high-traffic symbols available at `ntx::` for convenience.
pub use ntx_network::{Nic, nic::AfPacketNic};
