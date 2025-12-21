use std::hash::{Hash, Hasher};
use std::time::Instant;

use crate::packet::layers::{Ipv4, Udp};
use crate::resources::{ResourceId, ResourcePools};
use crate::socket::{PacketView, UdpRxContext};
use crate::stack::ReplyFrame;
use crate::traffic::udp_echo::UdpReplyTemplate;
use crate::{Ipv4Addr, MacAddr};
use std::collections::HashMap;

use super::core::{ConnEntry, ConnTableCore, HasSockId};

#[derive(Debug)]
pub enum UdpSocketError {
    MissingBinding(&'static str),
    MissingLocalIpv4Rid { sock_id: u64, rid: ResourceId },
    MissingLocalMacRid { sock_id: u64, rid: ResourceId },
    MissingLocalPortRid { sock_id: u64, rid: ResourceId },
    PayloadTooLarge(usize),
    UnknownSockId(u64),
    Build(anyhow::Error),
}

impl std::fmt::Display for UdpSocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBinding(what) => write!(f, "udp socket missing binding: {what}"),
            Self::MissingLocalIpv4Rid { sock_id, rid } => {
                write!(
                    f,
                    "udp socket missing local ipv4 rid for sock_id={sock_id}: {rid}"
                )
            }
            Self::MissingLocalMacRid { sock_id, rid } => {
                write!(
                    f,
                    "udp socket missing local mac rid for sock_id={sock_id}: {rid}"
                )
            }
            Self::MissingLocalPortRid { sock_id, rid } => {
                write!(
                    f,
                    "udp socket missing local port rid for sock_id={sock_id}: {rid}"
                )
            }
            Self::PayloadTooLarge(n) => write!(f, "udp socket payload too large: {n}"),
            Self::UnknownSockId(id) => write!(f, "udp socket unknown sock_id: {id}"),
            Self::Build(e) => write!(f, "udp socket build error: {e}"),
        }
    }
}

/// Socket binding information required to send UDP frames for a given `sock_id`.
///
/// This is the minimal set of fields needed to build an L2+L3+L4 reply template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpBinding {
    pub peer_ip: Ipv4Addr,
    pub peer_port: u16,
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
    pub peer_mac: MacAddr,
    pub local_mac: MacAddr,
    pub ttl: u8,
}

/// ResourceId-based binding for a UDP socket.
///
/// This is the control-plane friendly representation: all fields are `ResourceId`s.
/// Concrete values are resolved from `resources::ResourcePools` when finalizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpResourceBinding {
    pub local_ipv4: ResourceId,
    pub local_mac: ResourceId,
    pub local_udp_port: ResourceId,

    pub peer_ipv4: Ipv4Addr,
    pub peer_mac: MacAddr,
    pub peer_udp_port: u16,

    pub ttl: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PartialUdpResourceBinding {
    local_ipv4: Option<ResourceId>,
    local_mac: Option<ResourceId>,
    local_udp_port: Option<ResourceId>,
    peer_ipv4: Option<Ipv4Addr>,
    peer_mac: Option<MacAddr>,
    peer_udp_port: Option<u16>,
    ttl: Option<u8>,
}

/// A helper that manages resource-id based socket bindings.
///
/// This deliberately lives outside of `Table` so we don't have to change the generic
/// conn table container to store protocol-specific user state.
#[derive(Debug, Default)]
pub struct UdpSocketBinder {
    bindings: HashMap<u64, PartialUdpResourceBinding>,
}

impl UdpSocketBinder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind_local_ipv4_rid(&mut self, sock_id: u64, rid: ResourceId) {
        self.bindings.entry(sock_id).or_default().local_ipv4 = Some(rid);
    }

    pub fn bind_local_mac_rid(&mut self, sock_id: u64, rid: ResourceId) {
        self.bindings.entry(sock_id).or_default().local_mac = Some(rid);
    }

    pub fn bind_local_udp_port_rid(&mut self, sock_id: u64, rid: ResourceId) {
        self.bindings.entry(sock_id).or_default().local_udp_port = Some(rid);
    }

    pub fn bind_peer(
        &mut self,
        sock_id: u64,
        peer_ipv4: Ipv4Addr,
        peer_udp_port: u16,
        peer_mac: MacAddr,
    ) {
        let b = self.bindings.entry(sock_id).or_default();
        b.peer_ipv4 = Some(peer_ipv4);
        b.peer_udp_port = Some(peer_udp_port);
        b.peer_mac = Some(peer_mac);
    }

    pub fn bind_ttl(&mut self, sock_id: u64, ttl: u8) {
        self.bindings.entry(sock_id).or_default().ttl = Some(ttl);
    }

    /// Finalize bindings for this sock_id, resolving `ResourceId -> value` via ResourcePools,
    /// and inserting/updating the UDP conn entry.
    pub fn finalize_into_table(
        &self,
        pools: &ResourcePools,
        table: &mut Table,
        sock_id: u64,
    ) -> Result<(), UdpSocketError> {
        let Some(p) = self.bindings.get(&sock_id).cloned() else {
            return Err(UdpSocketError::MissingBinding("binding state"));
        };

        let local_ipv4_rid = p
            .local_ipv4
            .ok_or(UdpSocketError::MissingBinding("local_ipv4"))?;
        let local_mac_rid = p
            .local_mac
            .ok_or(UdpSocketError::MissingBinding("local_mac"))?;
        let local_port_rid = p
            .local_udp_port
            .ok_or(UdpSocketError::MissingBinding("local_udp_port"))?;

        let peer_ipv4 = p
            .peer_ipv4
            .ok_or(UdpSocketError::MissingBinding("peer_ipv4"))?;
        let peer_mac = p
            .peer_mac
            .ok_or(UdpSocketError::MissingBinding("peer_mac"))?;
        let peer_port = p
            .peer_udp_port
            .ok_or(UdpSocketError::MissingBinding("peer_udp_port"))?;

        let ttl = p.ttl.unwrap_or(64);

        let local_ip = pools
            .resolve_non_socket(crate::resources::ResourceKind::Ipv4, &local_ipv4_rid)
            .and_then(|v| match v {
                crate::resources::NonSocketResourceValue::Ipv4(ip) => Some(ip),
                _ => None,
            })
            .ok_or(UdpSocketError::MissingLocalIpv4Rid {
                sock_id,
                rid: local_ipv4_rid,
            })?;

        let local_mac = pools
            .resolve_non_socket(crate::resources::ResourceKind::Mac, &local_mac_rid)
            .and_then(|v| match v {
                crate::resources::NonSocketResourceValue::Mac(mac) => Some(mac),
                _ => None,
            })
            .ok_or(UdpSocketError::MissingLocalMacRid {
                sock_id,
                rid: local_mac_rid,
            })?;

        let local_port = pools
            .resolve_non_socket(crate::resources::ResourceKind::UdpPort, &local_port_rid)
            .and_then(|v| match v {
                crate::resources::NonSocketResourceValue::UdpPort(p) => Some(p),
                _ => None,
            })
            .ok_or(UdpSocketError::MissingLocalPortRid {
                sock_id,
                rid: local_port_rid,
            })?;

        table.bind_sock_id(
            sock_id,
            UdpBinding {
                peer_ip: peer_ipv4,
                peer_port,
                local_ip: Ipv4Addr(local_ip.octets()),
                local_port,
                peer_mac,
                local_mac,
                ttl,
            },
        );

        Ok(())
    }
}

/// Control-plane helper: create a new UDP socket.
///
/// Returns `(owner_id, sock_id)`:
/// - `owner_id` (`ResourceId`) is the control-plane identity used as `resources::OwnerId`.
/// - `sock_id` (`u64`) is the dataplane id used to reference bindings/conn-table entries.
///
/// Typical flow:
/// 1. `(owner, sock) = create_udp_socket(pools, &table, name)`
/// 2. allocate resources in pools using `owner` (ipv4/mac/udp_port)
/// 3. bind by resource ids using `UdpSocketBinder` keyed by `sock`
/// 4. `finalize_into_table(pools, &mut table, sock)`
#[inline]
pub fn create_udp_socket(
    pools: &mut ResourcePools,
    table: &Table,
    name: impl Into<String>,
) -> (ResourceId, u64) {
    let owner = pools.acquire_socket_owner(name);
    let sock_id = table.create_sock_id();
    // Optional strong association: record sock_id back into the socket registry entry.
    pools.registry_mut().set_socket_sock_id(&owner, sock_id);
    (owner, sock_id)
}

/// Back-compat helper if callers only need the socket owner id.
#[inline]
pub fn create_udp_socket_owner(pools: &mut ResourcePools, name: impl Into<String>) -> ResourceId {
    pools.acquire_socket_owner(name)
}

impl std::error::Error for UdpSocketError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Build(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

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
    /// Create a new UDP socket id with no binding yet.
    ///
    /// This is a pure userspace control-plane operation.
    /// Call `bind_sock_id` to associate the id with a concrete 4-tuple + MACs.
    #[inline]
    pub fn create_sock_id(&self) -> u64 {
        crate::resources::alloc_sock_id()
    }

    /// Bind an existing `sock_id` to a concrete UDP tuple + L2 addresses.
    ///
    /// This inserts/updates a connection entry such that future `send_with_sock_id`
    /// calls can build an Ethernet+IPv4+UDP reply frame.
    pub fn bind_sock_id(&mut self, sock_id: u64, binding: UdpBinding) {
        let key = Key {
            id: sock_id,
            peer_ip: binding.peer_ip,
            peer_port: binding.peer_port,
            local_ip: binding.local_ip,
            local_port: binding.local_port,
        };

        let now = Instant::now();
        let (entry, _inserted) = self.upsert_with_sock_id(key, || {
            let tpl = UdpReplyTemplate {
                eth: crate::EthernetHeader {
                    dst: binding.peer_mac,
                    src: binding.local_mac,
                    ethertype: crate::ETH_TYPE_IPV4,
                },
                ip: crate::Ipv4Header {
                    src: binding.local_ip,
                    dst: binding.peer_ip,
                    protocol: 17,
                    ttl: binding.ttl,
                    identification: 0,
                    flags_fragment: 0,
                },
                udp: crate::UdpHeader {
                    src_port: binding.local_port,
                    dst_port: binding.peer_port,
                },
            };

            Conn {
                key,
                reply: tpl,
                created_at: now,
                last_seen: now,
            }
        });
        entry.set_last_seen(now);
        let _ = entry;
    }

    /// Build a UDP reply frame for `payload` using the socket binding identified by `sock_id`.
    ///
    /// This does not transmit the frame; it only returns a `ReplyFrame` which callers
    /// may send via a NIC backend.
    pub fn build_reply_for_sock_id(
        &mut self,
        sock_id: u64,
        payload: &[u8],
    ) -> Result<ReplyFrame, UdpSocketError> {
        // No hard limit required here, but protect against pathological allocations.
        // Ethernet+IPv4+UDP headers are small; payload dominates.
        if payload.len() > 65507 {
            // 65535 - 20 (ipv4) - 8 (udp)
            return Err(UdpSocketError::PayloadTooLarge(payload.len()));
        }

        let conn = self
            .iter()
            .map(|(_k, c)| c)
            .find(|c| c.key.id == sock_id)
            .ok_or(UdpSocketError::UnknownSockId(sock_id))?;

        conn.build_reply(payload).map_err(UdpSocketError::Build)
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConnTableConfig;
    use crate::resources::{NonSocketResourceValue, ResourceKind};
    use crate::resources::{ResourcePools, ResourcePoolsConfig};

    #[test]
    fn udp_socket_create_bind_and_build_reply() {
        let mut table = Table::new(ConnTableConfig {
            max_entries: 128,
            ttl: None,
        });

        let id = table.create_sock_id();
        table.bind_sock_id(
            id,
            UdpBinding {
                peer_ip: crate::Ipv4Addr([10, 0, 0, 2]),
                peer_port: 12345,
                local_ip: crate::Ipv4Addr([10, 0, 0, 1]),
                local_port: 10000,
                peer_mac: crate::MacAddr([1, 2, 3, 4, 5, 6]),
                local_mac: crate::MacAddr([6, 5, 4, 3, 2, 1]),
                ttl: 64,
            },
        );

        let payload = b"hello";
        let frame = table
            .build_reply_for_sock_id(id, payload)
            .expect("build reply");

        // Sanity check: payload appears at the end of the frame.
        assert!(frame.bytes.ends_with(payload));
    }

    #[test]
    fn udp_socket_bind_by_resource_id_then_finalize() {
        // Build in-memory pools (no IO) and allocate pinned resources for one owner.
        let mut cfg = ResourcePoolsConfig::new();
        cfg.ipv4.push(crate::resources::Ipv4PoolConfig {
            name: "default".to_string(),
            cidr: "10.0.0.0/30".to_string(),
            exclude: vec![],
        });
        cfg.mac.push(crate::resources::MacPoolConfig {
            name: "default".to_string(),
            start: "02:00:00:00:00:10".to_string(),
            end: "02:00:00:00:00:10".to_string(),
            exclude: vec![],
        });
        cfg.udp_port.push(crate::resources::PortPoolConfig {
            name: "default".to_string(),
            protocol: None,
            start: 10000,
            end: 10000,
            exclude: vec![],
        });

        let mut pools: ResourcePools = cfg.build().expect("build pools");
        let owner = pools.alloc_socket_owner("s1");

        let (ipv4_rid, _ip) = {
            let (rid, v) = pools
                .acquire_and_pin_non_socket(ResourceKind::Ipv4, "default", owner, None)
                .expect("alloc ipv4");
            let NonSocketResourceValue::Ipv4(ip) = v else {
                unreachable!("resource kind/value mismatch")
            };
            (rid, ip)
        };
        let (mac_rid, _mac) = {
            let (rid, v) = pools
                .acquire_and_pin_non_socket(ResourceKind::Mac, "default", owner, None)
                .expect("alloc mac");
            let NonSocketResourceValue::Mac(mac) = v else {
                unreachable!("resource kind/value mismatch")
            };
            (rid, mac)
        };
        let (udp_rid, _port) = {
            let (rid, v) = pools
                .acquire_and_pin_non_socket(ResourceKind::UdpPort, "default", owner, None)
                .expect("alloc udp port");
            let NonSocketResourceValue::UdpPort(p) = v else {
                unreachable!("resource kind/value mismatch")
            };
            (rid, p)
        };

        let mut table = Table::new(ConnTableConfig {
            max_entries: 128,
            ttl: None,
        });

        let sock_id = table.create_sock_id();

        let mut binder = UdpSocketBinder::new();
        binder.bind_local_ipv4_rid(sock_id, ipv4_rid);
        binder.bind_local_mac_rid(sock_id, mac_rid);
        binder.bind_local_udp_port_rid(sock_id, udp_rid);
        binder.bind_peer(
            sock_id,
            crate::Ipv4Addr([10, 0, 0, 2]),
            12345,
            crate::MacAddr([1, 2, 3, 4, 5, 6]),
        );
        binder.bind_ttl(sock_id, 64);

        binder
            .finalize_into_table(&pools, &mut table, sock_id)
            .expect("finalize");

        let frame = table
            .build_reply_for_sock_id(sock_id, b"hello")
            .expect("build reply");
        assert!(frame.bytes.ends_with(b"hello"));
    }

    #[test]
    fn udp_create_socket_allocates_owner_id() {
        let mut cfg = ResourcePoolsConfig::new();
        cfg.ipv4.push(crate::resources::Ipv4PoolConfig {
            name: "default".to_string(),
            cidr: "10.0.0.0/30".to_string(),
            exclude: vec![],
        });

        let mut pools: ResourcePools = cfg.build().expect("build pools");

        let table = Table::new(ConnTableConfig {
            max_entries: 128,
            ttl: None,
        });

        let (owner, sock_id) = create_udp_socket(&mut pools, &table, "udp-s1");
        assert_eq!(pools.registry().socket_info(&owner).unwrap().name, "udp-s1");
        assert!(sock_id > 0);
        assert_eq!(
            pools.registry().socket_info(&owner).unwrap().sock_id,
            Some(sock_id)
        );

        // Owner id can be used to allocate resources.
        let (rid, v) = pools
            .acquire_and_pin_non_socket(
                crate::resources::ResourceKind::Ipv4,
                "default",
                owner,
                Some(sock_id),
            )
            .expect("alloc ipv4");
        let crate::resources::NonSocketResourceValue::Ipv4(ip) = v else {
            unreachable!("resource kind/value mismatch")
        };
        assert_eq!(ip, std::net::Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(pools.registry().using_sock_id_of(&rid), Some(sock_id));
    }

    #[test]
    fn udp_socket_build_reply_unknown_id_errors() {
        let mut table = Table::new(ConnTableConfig {
            max_entries: 128,
            ttl: None,
        });

        let err = table
            .build_reply_for_sock_id(9999, b"x")
            .expect_err("should error");
        match err {
            UdpSocketError::UnknownSockId(9999) => {}
            other => panic!("unexpected error: {other}"),
        }
    }
}
