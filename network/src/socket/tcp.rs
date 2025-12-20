use std::time::Instant;

use crate::Ipv4Addr;
use crate::packet::layers::{Ipv4, Tcp};
use crate::socket::TimeContext;
use crate::stack::ParsedPacket;

use super::core::{ConnEntry, ConnTableCore};

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

impl Table {
    /// 统一“收包入口”（TCP skeleton）。
    ///
    /// 目前 TCP socket 仍是骨架实现：这里仅做 key 提取（若可用）、插入/刷新 last_seen。
    ///
    /// 说明：等 TCP state machine 落地后，这里会演进为 SYN/ACK 等状态驱动的入口。
    pub fn on_rx(&mut self, pkt: &ParsedPacket<'_>, ctx: &TimeContext) -> anyhow::Result<&Conn> {
        let ip = pkt
            .get::<Ipv4>()
            .ok_or_else(|| anyhow::anyhow!("missing ipv4"))?;
        let tcp = pkt
            .get::<Tcp>()
            .ok_or_else(|| anyhow::anyhow!("missing tcp"))?;

        let key = Key {
            peer_ip: ip.src,
            peer_port: tcp.src_port,
            local_ip: ip.dst,
            local_port: tcp.dst_port,
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
