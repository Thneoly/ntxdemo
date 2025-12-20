//! Common imports for examples and quick prototyping.
//!
//! Usage:
//! ```ignore
//! use ntx::network::prelude::*;
//! ```

// Layering / builder DSL
pub use crate::stack::{Chain, LayerPkt, PacketBuilder, Raw, layers};

// NIC trait is frequently used in host-side examples.
#[cfg(feature = "host")]
pub use crate::Nic;

// Resource pools (config-driven).
pub use crate::resources::{ResourcePools, ResourcePoolsConfig};
