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

mod dsl;
mod graph;
mod layer;
pub use crate::packet::layers;
mod parser;
mod pipeline;
mod registry;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod dsl_tests;

// Curated public surface (avoid `pub use ...::*` to prevent accidental API leaks).
//
// If you need something that's not exported here, prefer importing it from its
// defining submodule (e.g. `stack::pipeline::ParsedPacket`).

// Core types
pub use layer::{AcceptResult, Layer, LayerId, LayerInstance, PacketContext};
pub use pipeline::{Action, PacketHandler, ParsedPacket, Pipeline, ReplyFrame};
pub use registry::LayerRegistry;

// Registry binding keys (used by layer registration glue).
pub use registry::BindKey;

// Graph plumbing (used by parse graph + docs).
pub use graph::{EdgeKind, PacketGraph};

// Parsing helpers
pub use parser::{
    build_packet, build_packet_no_payload, build_packet_no_payload_with_glue,
    build_packet_with_glue, parse_packet, parse_packet_graph, parse_packet_with_ctx,
    parse_packet_with_spans,
};

// Registry helpers
pub use pipeline::default_registry;

// Intentionally do NOT re-export echo/reply helpers from `stack`.
// Use `crate::traffic::udp_echo` instead.

// DSL remains part of the public surface (used by prelude).
pub use dsl::{Chain, LayerPkt, PacketBuilder, Raw};

// Also expose the `li` helper module path used by DSL internals.
pub use layer::li;

// DSL/graph are now intentionally not part of the default public surface.
