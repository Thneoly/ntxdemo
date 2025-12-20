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

use std::time::Instant;

use crate::{Ipv4Addr, MacAddr};

/// Per-packet monotonic time.
#[derive(Debug, Clone, Copy)]
pub struct TimeContext {
    pub now: Instant,
}

impl TimeContext {
    #[inline]
    pub fn new() -> Self {
        Self {
            now: Instant::now(),
        }
    }

    #[inline]
    pub fn with_now(now: Instant) -> Self {
        Self { now }
    }
}

/// Local receive identity for this packet path.
#[derive(Debug, Clone, Copy)]
pub struct LocalIdentity {
    pub local_mac: MacAddr,
    pub local_ip: Ipv4Addr,
}

impl LocalIdentity {
    #[inline]
    pub fn new(local_mac: MacAddr, local_ip: Ipv4Addr) -> Self {
        Self {
            local_mac,
            local_ip,
        }
    }
}

// No `BaseRxContext`:
//
// We keep RX context modelling as small orthogonal pieces (`TimeContext`,
// `LocalIdentity`, plus protocol-specific fields) and let call sites compose
// what they need.

/// UDP-specific RX context.
#[derive(Debug, Clone, Copy)]
pub struct UdpRxContext {
    pub time: TimeContext,
    pub local: LocalIdentity,
    pub local_port: u16,
}

impl UdpRxContext {
    #[inline]
    pub fn new(time: TimeContext, local: LocalIdentity, local_port: u16) -> Self {
        Self {
            time,
            local,
            local_port,
        }
    }

    #[inline]
    pub fn now(&self) -> Instant {
        self.time.now
    }

    #[inline]
    pub fn local_mac(&self) -> MacAddr {
        self.local.local_mac
    }

    #[inline]
    pub fn local_ip(&self) -> Ipv4Addr {
        self.local.local_ip
    }
}

/// A minimal protocol-agnostic parsed packet view for socket tables.
///
/// 目的：让 `socket::*` 不必在 public API 上绑定 `stack::ParsedPacket` 这个类型。
/// 目前我们只需要 `get::<T>()` 能取到已解析的 layer。
///
/// 注意：这是 socket 层的“最小契约”。如果未来 socket 需要更多能力（payload/原始 bytes），
/// 再在这里扩展，而不是反向依赖 stack。
pub trait PacketView {
    fn get<T: 'static>(&self) -> Option<&T>;

    /// Downcast hook for optional capabilities.
    fn as_any(&self) -> &(dyn std::any::Any + '_);

    /// Optional payload access (only some views can provide this).
    fn payload(&self) -> Option<&[u8]> {
        None
    }
}

impl PacketView for crate::stack::ParsedPacket<'_> {
    #[inline]
    fn get<T: 'static>(&self) -> Option<&T> {
        crate::stack::ParsedPacket::get(self)
    }

    #[inline]
    fn as_any(&self) -> &(dyn std::any::Any + '_) {
        self
    }

    #[inline]
    fn payload(&self) -> Option<&[u8]> {
        Some(crate::stack::ParsedPacket::payload(self))
    }
}

// Core surface.
pub use core::{ConnEntry, ConnKey, ConnTableConfig, ConnTableCore, ConnTableStats};

// Intentionally no protocol-specific re-exports here.
//
// Use `socket::<proto>::{Key, Conn, Table}` (e.g. `socket::udp::Table`) for a
// uniform surface across protocols.
