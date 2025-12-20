use std::time::Instant;

use crate::Ipv4Addr;

use super::core::{ConnEntry, ConnKey, ConnTableCore};

/// A minimal key for RAW IPv4 flows.
///
/// The typical discriminator for raw sockets is (proto, peer/local ip).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub protocol: u8,
    pub peer_ip: Ipv4Addr,
    pub local_ip: Ipv4Addr,
}

impl ConnKey for Key {
    fn proto_name(&self) -> &'static str {
        "raw-ip"
    }
}

/// A minimal RAW IPv4 connection-ish entry (skeleton).
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

/// Skeleton table for RAW IPv4.
#[allow(dead_code)]
pub type Table = ConnTableCore<Conn>;
