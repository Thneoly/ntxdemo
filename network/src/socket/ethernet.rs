use std::time::Instant;

use crate::MacAddr;

use super::core::{ConnEntry, ConnKey, ConnTableCore};

/// A minimal key for L2 ethernet-ish flows.
///
/// Includes ethertype so the same MAC pair can be multiplexed.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub peer_mac: MacAddr,
    pub local_mac: MacAddr,
    pub ethertype: u16,
}

impl std::hash::Hash for Key {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.peer_mac.0.hash(state);
        self.local_mac.0.hash(state);
        self.ethertype.hash(state);
    }
}

impl ConnKey for Key {
    fn proto_name(&self) -> &'static str {
        "eth"
    }
}

/// A minimal Ethernet connection-ish entry (skeleton).
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

/// Skeleton table for Ethernet.
#[allow(dead_code)]
pub type Table = ConnTableCore<Conn>;
