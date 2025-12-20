//! Guestnet-facing, WIT-friendly wrappers.
//!
//! Goal
//! ----
//! Provide a higher-level surface area that is convenient for `plugins/ntx-guestnet`
//! (and other WIT-facing components) to consume.
//!
//! Design constraints
//! ------------------
//! - Keep this module *pure* (no direct host NIC/AF_PACKET dependencies).
//! - Prefer small, serde-friendly POD structs.
//! - Provide stable data types for WIT bindings and socket-like APIs.
//!
//! Status
//! ------
//! This is currently a small re-export layer plus a few helper types.
//! Protocol-specific guestnet workflows should be built on top of:
//! - `socket::ConnTableCore` (+ per-protocol tables)
//! - `stack::ParsedPacket` pipeline

pub use crate::packet::headers::{Ipv4Addr, MacAddr};

/// A WIT-friendly representation of an L2 endpoint.
///
/// This is intentionally minimal: guestnet can carry this around on a socket
/// and inject it into TX without needing ownership of NIC-layer structures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct L2Endpoint {
    pub peer_mac: MacAddr,
    pub local_mac: MacAddr,
}

/// A WIT-friendly socket identifier.
///
/// Guestnet currently has its own socket ID space; we mirror that here to avoid
/// leaking internal table indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SocketId(pub u32);
