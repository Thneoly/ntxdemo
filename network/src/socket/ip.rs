use std::time::Instant;

use crate::Ipv4Addr;
use crate::packet::layers::Ipv4;
use crate::socket::TimeContext;
use crate::stack::ParsedPacket;

use super::core::{ConnEntry, ConnTableCore};

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

impl Table {
    /// 统一“收包入口”（RAW IPv4 skeleton）。
    pub fn on_rx(&mut self, pkt: &ParsedPacket<'_>, ctx: &TimeContext) -> anyhow::Result<&Conn> {
        let ip = pkt
            .get::<Ipv4>()
            .ok_or_else(|| anyhow::anyhow!("missing ipv4"))?;

        let key = Key {
            protocol: ip.proto,
            peer_ip: ip.src,
            local_ip: ip.dst,
        };

        let now = ctx.now;
        let (entry, _inserted) = self.upsert_with(key, || Conn {
            key,
            created_at: now,
            last_seen: now,
        });
        entry.set_last_seen(now);
        Ok(&*entry)
    }
}
