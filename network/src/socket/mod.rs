//! Socket/connection tables.
//!
//! Goal
//! ----
//! Provide a *stable network-layer capability* to retain per-peer/per-flow metadata
//! once the peer is known, so that:
//! - servers can reply "along the original route" without rebuilding headers every time
//! - clients can correlate incoming replies to an existing "socket/connection"
//! - higher layers can share one consistent abstraction across UDP/TCP/RAW/ETH
//!
//! Design notes
//! ------------
//! This is intentionally lightweight and userspace-friendly:
//! - No blocking; tables are pure data structures.
//! - No implicit timers; eviction is caller-driven via policy (max entries / ttl).
//! - Types are small/copyable where possible.
//!
//! The initial implementation focuses on UDP because echo workloads need it most.
//! TCP/RAW/ETH tables are provided as compile-safe skeletons so the API can converge.

mod core;
pub mod ethernet;
pub mod ip;
pub mod tcp;
pub mod udp;

// Core surface.
pub use core::{ConnEntry, ConnKey, ConnTableConfig, ConnTableCore, ConnTableStats};

// Intentionally no protocol-specific re-exports here.
//
// Use `socket::<proto>::{Key, Conn, Table}` (e.g. `socket::udp::Table`) for a
// uniform surface across protocols.
