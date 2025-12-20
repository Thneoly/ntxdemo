use std::time::Instant;

use crate::MacAddr;
use crate::packet::layers::Ether;
use crate::socket::TimeContext;
use crate::stack::ParsedPacket;

use super::core::{ConnEntry, ConnTableCore};

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

impl Table {
    /// 统一“收包入口”（Ethernet skeleton）。
    pub fn on_rx(&mut self, pkt: &ParsedPacket<'_>, ctx: &TimeContext) -> anyhow::Result<&Conn> {
        let eth = pkt
            .get::<Ether>()
            .ok_or_else(|| anyhow::anyhow!("missing ether"))?;

        let key = Key {
            peer_mac: eth.src,
            local_mac: eth.dst,
            ethertype: eth.ethertype,
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
