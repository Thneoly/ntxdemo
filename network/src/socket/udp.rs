use std::hash::{Hash, Hasher};
use std::time::Instant;

use crate::packet::layers::{Ipv4, Udp};
use crate::socket::{PacketView, UdpRxContext};
use crate::stack::ReplyFrame;
use crate::traffic::udp_echo::UdpReplyTemplate;
use crate::{Ipv4Addr, MacAddr};

use super::core::{ConnEntry, ConnTableCore, HasSockId};

/// UDP flow key used by the socket/conn-table layer.
///
/// This intentionally lives in `socket::udp` (not `stack`/`packet`) to avoid
/// cross-layer coupling. Conversion from parsed packets happens locally.
#[derive(Debug, Clone, Copy)]
pub struct Key {
    /// Monotonically increasing flow id (process-local).
    pub id: u64,
    pub peer_ip: Ipv4Addr,
    pub peer_port: u16,
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
}

impl PartialEq for Key {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.peer_ip == other.peer_ip
            && self.peer_port == other.peer_port
            && self.local_ip == other.local_ip
            && self.local_port == other.local_port
    }
}

impl Eq for Key {}

impl Hash for Key {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        // NOTE: scheme B - hash ignores `id`, only uses the 4-tuple.
        self.peer_ip.hash(state);
        self.peer_port.hash(state);
        self.local_ip.hash(state);
        self.local_port.hash(state);
    }
}

impl HasSockId for Key {
    #[inline]
    fn sock_id(&self) -> u64 {
        self.id
    }
}

impl Key {
    #[inline]
    pub fn from_parsed_packet(pkt: &impl PacketView) -> anyhow::Result<Self> {
        let ip = pkt
            .get::<Ipv4>()
            .ok_or_else(|| anyhow::anyhow!("missing ipv4"))?;
        let udp = pkt
            .get::<Udp>()
            .ok_or_else(|| anyhow::anyhow!("missing udp"))?;

        Ok(Self {
            id: crate::resources::alloc_sock_id(),
            peer_ip: ip.src,
            peer_port: udp.src_port,
            local_ip: ip.dst,
            local_port: udp.dst_port,
        })
    }
}

/// A connection-like entry for a UDP flow.
///
/// This ties together the peer/local 4-tuple plus a cached header template.
#[derive(Debug, Clone)]
pub struct Conn {
    pub key: Key,
    pub reply: UdpReplyTemplate,
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

impl Conn {
    /// Build a reply frame to the peer.
    #[inline]
    pub fn build_reply(&self, payload: &[u8]) -> anyhow::Result<ReplyFrame> {
        self.reply.build(payload)
    }
}

/// UDP specialization: a `ConnTableCore` storing `Conn` entries.
pub type Table = ConnTableCore<Conn>;

impl Table {
    /// 统一的“收包入口”：从 RX 包中提取 flow key、构建/刷新连接条目，并返回连接。
    ///
    /// 约定：
    /// - 该函数会刷新 `last_seen`
    /// - 缓存 miss 时会构建 `UdpReplyTemplate`（复用 traffic 层的模板构建）
    /// - 容器行为（淘汰、统计）由 `ConnTableCore` 处理
    #[inline]
    pub fn on_rx(&mut self, pkt: &impl PacketView, ctx: &UdpRxContext) -> anyhow::Result<&Conn> {
        let mut key = Key::from_parsed_packet(pkt)?;
        key.local_ip = ctx.local_ip();
        key.local_port = ctx.local_port;

        // 统一容器语义：查找/插入/统计/淘汰由 core 处理。
        // on_rx 承担“协议入口”职责：
        // - 负责把 RX 上的语义转换成 Conn（含 reply template）
        // - 负责 refresh last_seen
        let now = ctx.now();
        let (entry, _inserted) = self.upsert_with_sock_id(key, || {
            let tpl = UdpReplyTemplate::from_parsed_packet(pkt, ctx.local_mac())
                .expect("validated layers by Key::from_parsed_packet");
            Conn {
                key,
                reply: tpl,
                created_at: now,
                last_seen: now,
            }
        });
        entry.set_last_seen(now);
        Ok(&*entry)
    }

    /// Create a socket entry for a known peer tuple (client-style connect).
    ///
    /// This does not require any received packet; it constructs the reply template
    /// from tuple values.
    pub fn connect(
        &mut self,
        peer_ip: Ipv4Addr,
        peer_port: u16,
        local_ip: Ipv4Addr,
        local_port: u16,
        peer_mac: MacAddr,
        local_mac: MacAddr,
        ttl: u8,
    ) -> &Conn {
        let key = Key {
            id: crate::resources::alloc_sock_id(),
            peer_ip,
            peer_port,
            local_ip,
            local_port,
        };

        let now = Instant::now();
        let (entry, inserted) = self.upsert_with_sock_id(key, || {
            let tpl = UdpReplyTemplate {
                eth: crate::EthernetHeader {
                    dst: peer_mac,
                    src: local_mac,
                    ethertype: crate::ETH_TYPE_IPV4,
                },
                ip: crate::Ipv4Header {
                    src: local_ip,
                    dst: peer_ip,
                    protocol: 17,
                    ttl,
                    identification: 0,
                    flags_fragment: 0,
                },
                udp: crate::UdpHeader {
                    src_port: local_port,
                    dst_port: peer_port,
                },
            };

            Conn {
                key,
                reply: tpl,
                created_at: now,
                last_seen: now,
            }
        });

        // 保持与 on_rx 一致：每次“命中或插入”都刷新 last_seen。
        // created_at 仅在插入时由 closure 写入。
        let _ = inserted;
        entry.set_last_seen(now);
        &*entry
    }

    /// Client-style connect using an [`crate::ArpCache`] to resolve `peer_mac`.
    ///
    /// Contract:
    /// - Returns an error if the ARP cache does not contain a valid mapping for `peer_ip`.
    /// - On success, behaves like [`Table::connect`].
    pub fn connect_via_arp_cache(
        &mut self,
        arp: &mut crate::ArpCache,
        peer_ip: Ipv4Addr,
        peer_port: u16,
        local_ip: Ipv4Addr,
        local_port: u16,
        local_mac: MacAddr,
        ttl: u8,
    ) -> anyhow::Result<&Conn> {
        let peer_mac = arp
            .get(peer_ip)
            .ok_or_else(|| anyhow::anyhow!("arp cache miss for {}", crate::fmt_ipv4!(peer_ip)))?;

        Ok(self.connect(
            peer_ip, peer_port, local_ip, local_port, peer_mac, local_mac, ttl,
        ))
    }

    /// Find a socket id by destination/local address.
    ///
    /// This is a convenience helper for higher layers that only know the inbound
    /// packet's destination (local) ip/port and want to correlate it to an existing
    /// cached UDP "socket/connection".
    ///
    /// Notes:
    /// - This does a linear scan over the table (MVP-friendly). If we need this
    ///   long-term, we should add a dedicated index keyed by (local_ip, local_port).
    #[inline]
    pub fn sock_id_for_local(&self, local_ip: Ipv4Addr, local_port: Option<u16>) -> Option<u64> {
        self.iter()
            .map(|(_k, c)| c)
            .find(|c| {
                c.key.local_ip == local_ip
                    && local_port.map(|p| c.key.local_port == p).unwrap_or(true)
            })
            .map(|c| c.key.id)
    }
}
