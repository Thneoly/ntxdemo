//! Packet building blocks.
//!
//! This module is the "protocol surface" of the crate:
//! - `headers/*` are low-level, mostly stateless decode/encode helpers.
//! - `layers/*` are stack-specific `Layer` implementations used by the runtime registry.

pub mod headers;
pub mod layers;

// Intentionally avoid `pub use headers::*;` here.
//
// The crate root (`lib.rs`) already re-exports the stable header surface.
// Keeping `packet::headers::*` in its own namespace avoids name ambiguities with
// `stack::*` and keeps the public API more explicit.
