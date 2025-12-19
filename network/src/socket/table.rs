use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::stack::{ParsedPacket, ReplyFrame, UdpFlowKey, UdpReplyTemplate};
use crate::{Ipv4Addr, MacAddr};

/// A protocol-agnostic connection key.
///
/// This trait exists to let the socket layer grow beyond UDP (TCP/RAW/ETH) without
/// baking a single protocol's 4-tuple into the generic API surface.
#[allow(dead_code)]
pub trait ConnKey: Copy + Eq + std::hash::Hash {
    /// Human-readable protocol tag, mainly for debugging/metrics.
    fn proto_name(&self) -> &'static str;
}

impl ConnKey for UdpFlowKey {
    fn proto_name(&self) -> &'static str {
        "udp"
    }
}

/// A minimal 4-tuple key for TCP connections.
///
/// This is intentionally symmetric with `UdpFlowKey` so higher layers can use a
/// consistent concept, but TCP-specific semantics (state machine, seq/ack) are
/// *not* represented here.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TcpFlowKey {
    pub peer_ip: Ipv4Addr,
    pub peer_port: u16,
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
}

impl ConnKey for TcpFlowKey {
    fn proto_name(&self) -> &'static str {
        "tcp"
    }
}

/// A minimal key for RAW IPv4 flows.
///
/// The typical discriminator for raw sockets is (proto, peer/local ip). If your
/// use-case needs ports-like identifiers, use a different key type.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawIpKey {
    pub protocol: u8,
    pub peer_ip: Ipv4Addr,
    pub local_ip: Ipv4Addr,
}

impl ConnKey for RawIpKey {
    fn proto_name(&self) -> &'static str {
        "raw-ip"
    }
}

/// A minimal key for L2 ethernet-ish flows.
///
/// This is a placeholder for future L2 sockets. We include ethertype so the same
/// MAC pair can be multiplexed.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EthKey {
    pub peer_mac: MacAddr,
    pub local_mac: MacAddr,
    pub ethertype: u16,
}

impl std::hash::Hash for EthKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.peer_mac.0.hash(state);
        self.local_mac.0.hash(state);
        self.ethertype.hash(state);
    }
}

impl ConnKey for EthKey {
    fn proto_name(&self) -> &'static str {
        "eth"
    }
}

/// A protocol-agnostic connection entry.
///
/// The MVP common denominator across protocols is:
/// - a key
/// - liveness timestamps (for TTL/eviction)
///
/// Protocol-specific behavior (e.g. building a UDP reply frame) is provided by
/// per-protocol extension methods on the concrete type.
#[allow(dead_code)]
pub trait ConnEntry {
    type Key: ConnKey;

    fn key(&self) -> Self::Key;
    fn created_at(&self) -> Instant;
    fn last_seen(&self) -> Instant;
    fn set_last_seen(&mut self, now: Instant);
}

/// Generic stats for connection tables.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConnTableStats {
    pub lookups: u64,
    pub hits: u64,
    pub inserts: u64,
    pub evictions: u64,
}

/// Configuration for [`ConnTable`].
#[derive(Debug, Clone, Copy)]
pub struct ConnTableConfig {
    /// Maximum number of tracked sockets.
    pub max_entries: usize,
    /// Optional TTL; entries older than this are eligible for eviction on insert.
    pub ttl: Option<Duration>,
}

impl Default for ConnTableConfig {
    fn default() -> Self {
        Self {
            max_entries: 1024,
            ttl: Some(Duration::from_secs(60)),
        }
    }
}

/// A connection-like entry for a UDP flow.
///
/// The key difference from a pure "reply template" is that this struct represents a
/// *connection-ish* concept: it ties together the peer/local 4-tuple plus the cached
/// reverse-path header template.
#[derive(Debug, Clone)]
pub struct UdpSocket {
    pub key: UdpFlowKey,
    pub reply: UdpReplyTemplate,
    pub created_at: Instant,
    pub last_seen: Instant,
}

pub type Conn = UdpSocket;

impl ConnEntry for UdpSocket {
    type Key = UdpFlowKey;

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

/// A minimal TCP connection-ish entry (skeleton).
///
/// This intentionally does not include any TCP state machine yet.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TcpConn {
    pub key: TcpFlowKey,
    pub created_at: Instant,
    pub last_seen: Instant,
}

impl ConnEntry for TcpConn {
    type Key = TcpFlowKey;

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

/// A minimal RAW IPv4 connection-ish entry (skeleton).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RawIpConn {
    pub key: RawIpKey,
    pub created_at: Instant,
    pub last_seen: Instant,
}

impl ConnEntry for RawIpConn {
    type Key = RawIpKey;

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

/// A minimal Ethernet connection-ish entry (skeleton).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EthConn {
    pub key: EthKey,
    pub created_at: Instant,
    pub last_seen: Instant,
}

impl ConnEntry for EthConn {
    type Key = EthKey;

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

/// A protocol-agnostic connection table.
///
/// This type only implements generic behaviors shared across protocols:
/// - keyed lookup
/// - liveness refresh timestamps
/// - TTL/cap eviction
/// - generic stats
///
/// Per-protocol workflows (e.g. building UDP reply templates from RX packets, TCP state
/// machines, RAW socket semantics) should live in per-protocol impl blocks.
#[derive(Debug)]
pub struct ConnTableCore<C: ConnEntry> {
    cfg: ConnTableConfig,
    map: HashMap<C::Key, C>,
    stats: ConnTableStats,
}

impl<C: ConnEntry> Default for ConnTableCore<C> {
    fn default() -> Self {
        Self::new(ConnTableConfig::default())
    }
}

impl<C: ConnEntry> ConnTableCore<C> {
    pub fn new(cfg: ConnTableConfig) -> Self {
        Self {
            cfg,
            map: HashMap::new(),
            stats: ConnTableStats::default(),
        }
    }

    pub fn stats(&self) -> ConnTableStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn get(&mut self, key: &C::Key) -> Option<&C> {
        self.stats.lookups += 1;
        if self.map.contains_key(key) {
            self.stats.hits += 1;
        }
        self.map.get(key)
    }

    pub fn get_mut(&mut self, key: &C::Key) -> Option<&mut C> {
        self.stats.lookups += 1;
        if self.map.contains_key(key) {
            self.stats.hits += 1;
        }
        self.map.get_mut(key)
    }

    pub fn remove(&mut self, key: &C::Key) -> Option<C> {
        self.map.remove(key)
    }

    pub fn insert(&mut self, key: C::Key, value: C) -> Option<C> {
        self.evict_if_needed();
        let prev = self.map.insert(key, value);
        self.stats.inserts += 1;
        prev
    }

    pub fn contains_key(&self, key: &C::Key) -> bool {
        self.map.contains_key(key)
    }

    fn evict_if_needed(&mut self) {
        if let Some(ttl) = self.cfg.ttl {
            let now = Instant::now();
            let mut expired: Vec<C::Key> = Vec::new();
            for (k, v) in self.map.iter() {
                if now.duration_since(v.last_seen()) > ttl {
                    expired.push(*k);
                }
            }
            for k in expired {
                if self.map.remove(&k).is_some() {
                    self.stats.evictions += 1;
                }
            }
        }

        if self.map.len() >= self.cfg.max_entries {
            if let Some((oldest_key, _)) = self
                .map
                .iter()
                .min_by_key(|(_k, v)| v.last_seen())
                .map(|(k, v)| (*k, v.last_seen()))
            {
                if self.map.remove(&oldest_key).is_some() {
                    self.stats.evictions += 1;
                }
            }
        }
    }
}

/// UDP specialization: preserve the historical `ConnTable` API surface.
///
/// This keeps downstream users unchanged while letting other protocols implement
/// their own `*ConnTable` workflows on the same generic core.
/// UDP specialization: a `ConnTableCore` storing `UdpSocket` entries.
pub type UdpConnTable = ConnTableCore<UdpSocket>;

/// Back-compat: keep the public `ConnTable` name meaning UDP.
pub type ConnTable = UdpConnTable;

/// Skeleton tables for other protocols.
#[allow(dead_code)]
pub type TcpConnTable = ConnTableCore<TcpConn>;
#[allow(dead_code)]
pub type RawIpConnTable = ConnTableCore<RawIpConn>;
#[allow(dead_code)]
pub type EthConnTable = ConnTableCore<EthConn>;

impl UdpConnTable {
    /// Insert or refresh from a received packet.
    ///
    /// `my_*` lets the caller override "local" tuple fields when required.
    /// - For servers: `my_ip` should be the selected identity IP (request's dst)
    /// - For servers: `my_port` is the listening port
    /// - For clients: `my_ip/my_port` are the client's local tuple
    ///
    /// The reply template uses `my_mac` as the Ethernet source.
    pub fn get_or_insert_from_rx(
        &mut self,
        pkt: &ParsedPacket<'_>,
        my_ip: Ipv4Addr,
        my_port: u16,
        my_mac: MacAddr,
    ) -> anyhow::Result<&Conn> {
        let mut key = UdpFlowKey::from_parsed_packet(pkt)?;
        key.local_ip = my_ip;
        key.local_port = my_port;

        if self.map.contains_key(&key) {
            let now = Instant::now();
            let sock = self.map.get_mut(&key).expect("checked");
            sock.set_last_seen(now);
            self.stats.lookups += 1;
            self.stats.hits += 1;
            return Ok(self.map.get(&key).expect("exists"));
        }

        self.evict_if_needed();

        let tpl = UdpReplyTemplate::from_parsed_packet(pkt, my_mac)?;
        let now = Instant::now();
        let sock = Conn {
            key,
            reply: tpl,
            created_at: now,
            last_seen: now,
        };
        self.map.insert(key, sock);
        self.stats.inserts += 1;
        Ok(self.map.get(&key).expect("inserted"))
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
        let key = UdpFlowKey {
            peer_ip,
            peer_port,
            local_ip,
            local_port,
        };

        if self.map.contains_key(&key) {
            return self.map.get(&key).expect("exists");
        }

        self.evict_if_needed();

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

        let now = Instant::now();
        let sock = Conn {
            key,
            reply: tpl,
            created_at: now,
            last_seen: now,
        };
        self.map.insert(key, sock);
        self.stats.inserts += 1;
        self.map.get(&key).expect("inserted")
    }

    /// Client-style connect using an [`crate::ArpCache`] to resolve `peer_mac`.
    ///
    /// Contract:
    /// - Returns an error if the ARP cache does not contain a valid mapping for `peer_ip`.
    /// - On success, behaves like [`UdpConnTable::connect`].
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
            .ok_or_else(|| anyhow::anyhow!("arp cache miss for peer_ip"))?;
        Ok(self.connect(
            peer_ip, peer_port, local_ip, local_port, peer_mac, local_mac, ttl,
        ))
    }

    /// Convenience: build a packet for an existing key.
    pub fn build_for(&mut self, key: &UdpFlowKey, payload: &[u8]) -> anyhow::Result<ReplyFrame> {
        let sock = self
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("udp socket not found"))?;
        sock.build_reply(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::layers::{Ether, Ipv4, Udp};
    use crate::stack::{LayerId, LayerInstance};
    use std::collections::HashSet;

    #[test]
    fn udp_socket_table_inserts_and_hits() {
        let pkt = ParsedPacket {
            layers: vec![
                LayerInstance {
                    id: LayerId::Ether,
                    inner: Box::new(Ether {
                        dst: MacAddr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
                        src: MacAddr([0x10, 0x20, 0x30, 0x40, 0x50, 0x60]),
                        ethertype: crate::ETH_TYPE_IPV4,
                    }),
                },
                LayerInstance {
                    id: LayerId::Ipv4,
                    inner: Box::new(Ipv4 {
                        src: Ipv4Addr([10, 0, 0, 1]),
                        dst: Ipv4Addr([10, 0, 0, 2]),
                        proto: 17,
                        ttl: 64,
                        identification: 0,
                        flags_fragment: 0,
                        ihl_bytes: 20,
                    }),
                },
                LayerInstance {
                    id: LayerId::Udp,
                    inner: Box::new(Udp {
                        src_port: 1111,
                        dst_port: 7,
                        src_ip: Some(Ipv4Addr([10, 0, 0, 1])),
                        dst_ip: Some(Ipv4Addr([10, 0, 0, 2])),
                    }),
                },
            ],
            payload: b"hi",
        };

        let mut t = ConnTable::new(ConnTableConfig {
            max_entries: 8,
            ttl: None,
        });

        let my_ip = Ipv4Addr([10, 0, 0, 2]);
        let my_port = 7;
        let my_mac = MacAddr([1, 2, 3, 4, 5, 6]);

        let key = t
            .get_or_insert_from_rx(&pkt, my_ip, my_port, my_mac)
            .unwrap()
            .key;
        assert_eq!(t.len(), 1);

        let _ = t.build_for(&key, b"hi").unwrap();

        let key2 = t
            .get_or_insert_from_rx(&pkt, my_ip, my_port, my_mac)
            .unwrap()
            .key;
        assert_eq!(key2, key);

        let st = t.stats();
        assert!(st.inserts >= 1);
        assert!(st.hits >= 1);
    }

    #[test]
    fn udp_socket_table_connect_via_arp_cache() {
        let mut arp = crate::ArpCache::new(Duration::from_secs(60));
        let peer_ip = Ipv4Addr([10, 0, 0, 2]);
        let peer_mac = MacAddr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        arp.insert(peer_ip, peer_mac);

        let mut t = ConnTable::new(ConnTableConfig {
            max_entries: 8,
            ttl: None,
        });

        let s = t
            .connect_via_arp_cache(
                &mut arp,
                peer_ip,
                7,
                Ipv4Addr([10, 0, 0, 1]),
                1111,
                MacAddr([1, 2, 3, 4, 5, 6]),
                64,
            )
            .unwrap();

        assert_eq!(s.reply.eth.dst, peer_mac);
        assert_eq!(s.key.peer_ip, peer_ip);
        assert_eq!(s.key.peer_port, 7);
    }

    #[test]
    fn skeleton_tables_can_insert_and_lookup() {
        // TCP skeleton
        let mut tcp = TcpConnTable::new(ConnTableConfig {
            max_entries: 2,
            ttl: None,
        });
        let k1 = TcpFlowKey {
            peer_ip: Ipv4Addr([10, 0, 0, 2]),
            peer_port: 80,
            local_ip: Ipv4Addr([10, 0, 0, 1]),
            local_port: 40000,
        };
        let now = Instant::now();
        tcp.insert(
            k1,
            TcpConn {
                key: k1,
                created_at: now,
                last_seen: now,
            },
        );
        assert!(tcp.get(&k1).is_some());

        // RAW skeleton
        let mut raw = RawIpConnTable::new(ConnTableConfig {
            max_entries: 2,
            ttl: None,
        });
        let rk = RawIpKey {
            protocol: 1,
            peer_ip: Ipv4Addr([10, 0, 0, 2]),
            local_ip: Ipv4Addr([10, 0, 0, 1]),
        };
        raw.insert(
            rk,
            RawIpConn {
                key: rk,
                created_at: now,
                last_seen: now,
            },
        );
        assert!(raw.get(&rk).is_some());

        // ETH skeleton + hashability sanity
        let mut eth = EthConnTable::new(ConnTableConfig {
            max_entries: 2,
            ttl: None,
        });
        let ek = EthKey {
            peer_mac: MacAddr([1, 2, 3, 4, 5, 6]),
            local_mac: MacAddr([6, 5, 4, 3, 2, 1]),
            ethertype: crate::ETH_TYPE_IPV4,
        };
        eth.insert(
            ek,
            EthConn {
                key: ek,
                created_at: now,
                last_seen: now,
            },
        );
        assert!(eth.get(&ek).is_some());

        let mut hs = HashSet::new();
        hs.insert(ek);
        assert!(hs.contains(&ek));
    }
}
