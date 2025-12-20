//! Resource pool management (IP/MAC/Port pools) loaded from a config file.
//!
//! This module is intentionally small and standalone so it can be reused by examples
//! and higher-level components.
//!
//! Design goals:
//! - Deterministic allocation order (stable across runs).
//! - Simple ownership model: allocate → release.
//! - Config-driven: parse a config file at startup to define the available resources.

mod config;
mod ipv4;
mod mac;
mod named;
mod parse;
mod pools;
mod port;
mod publish;

pub use config::*;
pub use ipv4::*;
pub use mac::*;
pub use named::*;
pub use pools::*;
pub use port::*;
// Note: `publish` contains convenience helpers for examples; we don't glob re-export
// to avoid unused-import warnings when the helper isn't referenced.
