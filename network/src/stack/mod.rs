//! Userspace network stack building blocks.
//!
//! This module provides a Scapy-like, runtime-extensible packet layering system:
//!
//! - Packets are parsed into an ordered list of layers (`LayerInstance`).
//! - Next-layer selection is driven by a runtime registry (`LayerRegistry`).
//! - Handlers operate on a parsed packet view (`ParsedPacket`), not a fixed struct.
//!
//! The goal is to keep protocol processing independent from the underlying I/O backend
//! and make adding new protocol layers a registration-only operation.

mod graph;
mod layer;
pub use crate::packet::layers;
mod parser;
mod pipeline;
mod registry;

#[cfg(test)]
mod tests;

pub use graph::*;
pub use layer::*;
pub use parser::*;
pub use pipeline::*;
pub use registry::*;
