use std::time::Instant;

use crate::Ipv4Addr;

use super::core::{ConnEntry, ConnKey, ConnTableCore};

/// A minimal 4-tuple key for TCP connections.
///
/// TCP-specific semantics (state machine, seq/ack) are *not* represented here.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub peer_ip: Ipv4Addr,
    pub peer_port: u16,
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
}

impl ConnKey for Key {
    fn proto_name(&self) -> &'static str {
        "tcp"
    }
}

/// A minimal TCP connection-ish entry (skeleton).
///
/// This intentionally does not include any TCP state machine yet.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Conn {
    pub key: Key,
    pub created_at: Instant,
    pub last_seen: Instant,
}

impl ConnEntry for Conn {
    type Key = Key;

    fn key(&self) -> Self::Key {
        self.key
    }

    fn created_at(&self) -> Instant {
        self.created_at
    }

    fn last_seen(&self) -> Instant {
        self.last_seen
    }

    fn set_last_seen(&mut self, now: Instant) {
        self.last_seen = now;
    }
}

/// Skeleton table for TCP.
#[allow(dead_code)]
pub type Table = ConnTableCore<Conn>;
