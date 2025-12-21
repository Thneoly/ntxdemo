use anyhow::{Context, Result};
use ntx_network::{
    ArpCache, ConnTableConfig, abr, default_registry,
    nic::AfPacketNic,
    prelude::*,
    resources,
    socket::{
        self,
        udp::{self, Table, UdpSocketBinder},
    },
    stack::{
        LayerId, LayerRegistry, PacketContext,
        layers::{Ipv4, Udp},
        parse_packet_with_ctx,
    },
};
use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use tracing::error;

use crate::audit_registry;

/// Resource request options.
///
/// Kernel doesn't currently distinguish client/server; this interface is intended as a
/// control-plane hook to register/allocate resources and then refresh the ABR snapshot.
#[derive(Debug, Clone, Default)]
pub struct ResourceRequest {
    /// Socket owner id. If `None`, kernel will allocate a new socket owner id.
    ///
    /// This is a `resources::ResourceId` (UUID v4) and is used as `resources::OwnerId`
    /// for all subsequent resource allocations.
    pub owner: Option<resources::OwnerId>,

    /// Optional friendly name to store in the registry when `owner` is auto-allocated.
    pub owner_name: Option<String>,

    /// Pool names. If empty, defaults to "default".
    pub ipv4_pool: String,
    pub mac_pool: String,
    pub udp_port_pool: String,
    pub tcp_port_pool: String,

    /// Optional explicit pins.
    pub pin_ipv4: Option<std::net::Ipv4Addr>,
    pub pin_mac: Option<ntx_network::MacAddr>,
    pub pin_udp_ports: Vec<u16>,
    pub pin_tcp_ports: Vec<u16>,
}

/// Resource request result (allocated/pinned ids).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceResponse {
    pub owner: resources::OwnerId,
    pub ipv4: Option<(resources::ResourceId, std::net::Ipv4Addr)>,
    pub mac: Option<(resources::ResourceId, ntx_network::MacAddr)>,
    pub udp_ports: Vec<(resources::ResourceId, u16)>,
    pub tcp_ports: Vec<(resources::ResourceId, u16)>,
}

/// Minimal control-plane inputs needed to create and bind a UDP socket.
///
/// Notes:
/// - This is a pure control-plane operation: it only updates resource pools + UDP conn table.
/// - No NIC IO happens here.
#[derive(Debug, Clone)]
pub struct UdpSocketCreateRequest {
    /// Friendly name stored in registry.
    pub name: String,

    /// Pool names used for local resource acquisition. If empty, defaults to "default".
    pub ipv4_pool: String,
    pub mac_pool: String,
    pub udp_port_pool: String,

    /// Peer tuple (value-based).
    pub peer_ipv4: ntx_network::Ipv4Addr,
    pub peer_udp_port: u16,
    pub peer_mac: ntx_network::MacAddr,

    /// Defaults to 64 if not provided.
    pub ttl: Option<u8>,
}

/// Result of creating a UDP socket via kernel wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpSocketCreateResponse {
    pub owner: resources::ResourceId,
    pub sock_id: u64,

    pub local_ipv4_rid: resources::ResourceId,
    pub local_mac_rid: resources::ResourceId,
    pub local_udp_port_rid: resources::ResourceId,

    pub local_ipv4: std::net::Ipv4Addr,
    pub local_mac: ntx_network::MacAddr,
    pub local_udp_port: u16,
}
struct Kernel {
    config: Config,
    reg: LayerRegistry,
    pools: Mutex<ResourcePools>,
    store: Mutex<abr::BindingStore>,
    arp_cache: ArpCache,
    udp_sockets: Mutex<Table>,
    nic: Mutex<Box<dyn Nic + Send + Sync>>,

    abr_view: RwLock<Arc<abr::ResourceView>>,
}

static KERNEL: Lazy<Kernel> = Lazy::new(|| Kernel::new().expect("Kernel::new failed"));

#[derive(Debug, Clone, Deserialize)]
struct NicConfig {
    iface: String,
}
#[derive(Debug, Clone, Deserialize)]
struct ResourceConfig {
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    nic: NicConfig,
    resource: ResourceConfig,
}

impl Kernel {
    fn new() -> Result<Self> {
        let cfg = Config::load_yaml_file("config/config.yaml")
            .context("load kernel config (config/config.yaml)")?;
        let nic: Mutex<Box<dyn Nic + Send + Sync>> = Mutex::new(Box::new(
            AfPacketNic::open(&cfg.nic.iface).context("open afpacket nic")?,
        ));

        let reg: LayerRegistry = default_registry();
        let arp_cache = ArpCache::new(std::time::Duration::from_secs(60));
        let udp_sockets = Table::new(ConnTableConfig {
            max_entries: 4096,
            ttl: None,
        });
        let resource_pools_config = ResourcePoolsConfig::load_yaml_file(&cfg.resource.path)?;
        let pools = Mutex::new(resource_pools_config.build()?);
        let abr_store = Mutex::new(abr::BindingStore::default());

        // Load an initial ABR snapshot.
        let abr_view = abr::load_view();

        Ok(Self {
            config: cfg,
            reg,
            pools,
            store: abr_store,
            arp_cache,
            udp_sockets: Mutex::new(udp_sockets),
            nic,
            abr_view: RwLock::new(abr_view),
        })
    }
    fn nic(&self) -> parking_lot::MutexGuard<'_, Box<dyn Nic + Send + Sync>> {
        self.nic.lock()
    }
    fn reg(&self) -> &LayerRegistry {
        &self.reg
    }

    fn abr_view(&self) -> Arc<abr::ResourceView> {
        self.abr_view.read().clone()
    }

    fn refresh_abr_view(&self) {
        // Control-plane refresh: swap in the latest global ABR snapshot.
        let view = abr::load_view();
        *self.abr_view.write() = view;
    }
    fn udp_sockets(&self) -> parking_lot::MutexGuard<'_, Table> {
        self.udp_sockets.lock()
    }
}

impl Config {
    fn load_yaml_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read config file: {}", path.display()))?;
        let cfg: Self = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse yaml config: {}", path.display()))?;
        Ok(cfg)
    }
}

// init function to setup networking stack
pub fn init(_path: impl AsRef<Path>) -> Result<()> {
    let _ = &*KERNEL;
    Ok(())
}

/// Request/allocate resources from kernel control-plane.
///
/// This is intentionally minimal for now: it provides a single place where future
/// resource acquisition can hook in, and ensures the ABR snapshot is refreshed exactly
/// once after resources change.
pub fn request_resources(req: ResourceRequest) -> Result<ResourceResponse> {
    // Apply pins/allocations to the shared pools (control-plane).
    // Note: Pinning is deterministic and establishes ownership; dataplane uses ABR snapshot.
    let mut pools = KERNEL.pools.lock();

    let owner = req.owner.unwrap_or_else(|| {
        let name = req
            .owner_name
            .clone()
            .unwrap_or_else(|| "socket".to_string());
        pools.alloc_socket_owner(name)
    });

    // Step-2 audit / management registry: record owner name (best-effort).
    audit_registry::with_audit_registry_mut(|reg: &mut audit_registry::AuditRegistry| {
        reg.record_owner_name(owner, req.owner_name.clone());
    });

    let mut resp = ResourceResponse {
        owner,
        ..ResourceResponse::default()
    };

    let ipv4_pool = if req.ipv4_pool.is_empty() {
        "default"
    } else {
        req.ipv4_pool.as_str()
    };
    let mac_pool = if req.mac_pool.is_empty() {
        "default"
    } else {
        req.mac_pool.as_str()
    };
    let udp_pool = if req.udp_port_pool.is_empty() {
        "default"
    } else {
        req.udp_port_pool.as_str()
    };
    let tcp_pool = if req.tcp_port_pool.is_empty() {
        "default"
    } else {
        req.tcp_port_pool.as_str()
    };

    let wants_auto_acquire = req.pin_ipv4.is_none()
        && req.pin_mac.is_none()
        && req.pin_udp_ports.is_empty()
        && req.pin_tcp_ports.is_empty();

    // IPv4
    if let Some(ip) = req.pin_ipv4 {
        let rid = pools.pin_ipv4_with_id(ipv4_pool, resp.owner, ip, None)?;
        resp.ipv4 = Some((rid, ip));

        audit_registry::with_audit_registry_mut(|reg| reg.record_ipv4(resp.owner, rid, ip));
    } else if wants_auto_acquire {
        let (rid, ip) = pools.alloc_ipv4_for(ipv4_pool, resp.owner, None)?;
        resp.ipv4 = Some((rid, ip));

        audit_registry::with_audit_registry_mut(|reg| reg.record_ipv4(resp.owner, rid, ip));
    }

    // MAC
    if let Some(mac) = req.pin_mac {
        let rid = pools.pin_mac_with_id(mac_pool, resp.owner, mac, None)?;
        resp.mac = Some((rid, mac));

        audit_registry::with_audit_registry_mut(|reg| reg.record_mac(resp.owner, rid, mac));
    } else if wants_auto_acquire {
        let (rid, mac) = pools.alloc_mac_for(mac_pool, resp.owner, None)?;
        resp.mac = Some((rid, mac));

        audit_registry::with_audit_registry_mut(|reg| reg.record_mac(resp.owner, rid, mac));
    }

    // UDP ports
    if !req.pin_udp_ports.is_empty() {
        for p in &req.pin_udp_ports {
            let rid = pools.pin_udp_port_with_id(udp_pool, resp.owner, *p, None)?;
            resp.udp_ports.push((rid, *p));

            audit_registry::with_audit_registry_mut(|reg| reg.record_udp_port(resp.owner, rid, *p));
        }
    } else if wants_auto_acquire {
        let (rid, port) = pools.alloc_udp_port_for(udp_pool, resp.owner, None)?;
        resp.udp_ports.push((rid, port));

        audit_registry::with_audit_registry_mut(|reg| reg.record_udp_port(resp.owner, rid, port));
    }

    // TCP ports
    if !req.pin_tcp_ports.is_empty() {
        for p in &req.pin_tcp_ports {
            let rid = pools.pin_tcp_port_with_id(tcp_pool, resp.owner, *p, None)?;
            resp.tcp_ports.push((rid, *p));

            audit_registry::with_audit_registry_mut(|reg| reg.record_tcp_port(resp.owner, rid, *p));
        }
    } else if wants_auto_acquire {
        let (rid, port) = pools.alloc_tcp_port_for(tcp_pool, resp.owner, None)?;
        resp.tcp_ports.push((rid, port));

        audit_registry::with_audit_registry_mut(|reg| reg.record_tcp_port(resp.owner, rid, port));
    }

    // Publish ABR snapshot from pinned resources for this owner.
    let mut store = KERNEL.store.lock();
    pools.publish_abr_for_owner(&mut store, &resp.owner, abr::BindingOwner::KernelIface);

    // Refresh cached ABR view once.
    refresh_abr();
    Ok(resp)
}

/// Kernel-facing convenience wrapper: create a UDP socket and fully bind it.
///
/// This wires together:
/// - `udp::create_udp_socket` (owner + sock_id)
/// - resource allocation (IPv4/MAC/UDP port) with `using_sock_id` propagation
/// - `UdpSocketBinder` bind-by-ResourceId + finalize into the UDP conn-table
pub fn create_udp_socket(req: UdpSocketCreateRequest) -> Result<UdpSocketCreateResponse> {
    let ipv4_pool = if req.ipv4_pool.is_empty() {
        "default"
    } else {
        req.ipv4_pool.as_str()
    };
    let mac_pool = if req.mac_pool.is_empty() {
        "default"
    } else {
        req.mac_pool.as_str()
    };
    let udp_pool = if req.udp_port_pool.is_empty() {
        "default"
    } else {
        req.udp_port_pool.as_str()
    };

    let ttl = req.ttl.unwrap_or(64);

    // Step 1: create owner+sock_id, and allocate resources (control-plane).
    let mut pools = KERNEL.pools.lock();
    let mut table = KERNEL.udp_sockets.lock();
    let (owner, sock_id) = udp::create_udp_socket(&mut pools, &table, req.name);

    let (local_ipv4_rid, local_ipv4) = pools.alloc_ipv4_for_socket(ipv4_pool, owner)?;
    let (local_mac_rid, local_mac) = pools.alloc_mac_for_socket(mac_pool, owner)?;
    let (local_udp_port_rid, local_udp_port) = pools.alloc_udp_port_for_socket(udp_pool, owner)?;

    // Step 2: bind by ResourceId and finalize into table.
    let mut binder = UdpSocketBinder::new();
    binder.bind_local_ipv4_rid(sock_id, local_ipv4_rid);
    binder.bind_local_mac_rid(sock_id, local_mac_rid);
    binder.bind_local_udp_port_rid(sock_id, local_udp_port_rid);
    binder.bind_peer(sock_id, req.peer_ipv4, req.peer_udp_port, req.peer_mac);
    binder.bind_ttl(sock_id, ttl);
    binder.finalize_into_table(&pools, &mut table, sock_id)?;

    Ok(UdpSocketCreateResponse {
        owner,
        sock_id,
        local_ipv4_rid,
        local_mac_rid,
        local_udp_port_rid,
        local_ipv4,
        local_mac,
        local_udp_port,
    })
}

/// Refresh the cached ABR view from the global ABR snapshot.
///
/// Call this after control-plane changes (e.g. resources allocated/released).
pub fn refresh_abr() {
    KERNEL.refresh_abr_view();
}

pub fn non_blocking_recv() -> Option<Vec<u8>> {
    non_blocking_recv_with_sock().map(|(_sock, payload)| payload)
}

/// Receive one packet payload (non-blocking) and try to correlate it to a UDP socket.
///
/// Step-1 数据面策略：
/// - 从报文解析出 4-tuple
/// - 构造 `socket::udp::Key { id: 0, ... }`（Scheme B：Eq/Hash 忽略 id）
/// - 在 UDP conn table 里查找命中的 conn，并返回其 `key.id` 作为 sock_id
///
/// 返回：`Some((sock_id_opt, payload))` 或 `None`（无包）。
pub fn non_blocking_recv_with_sock() -> Option<(Option<resources::SockId>, Vec<u8>)> {
    let mut nic = KERNEL.nic();
    let reg = KERNEL.reg();
    let abr_view = KERNEL.abr_view();
    let udp_sockets = KERNEL.udp_sockets();

    // Kernel doesn't distinguish client/server, and identities may use multiple dst MACs;
    // disable L2 filtering here and let ABR decide relevance.
    let ctx = PacketContext {
        iface_mac: None,
        abr: Some(abr_view),
    };

    let mut buf = vec![0u8; 65536];
    let n = match nic.recv_nonblocking(&mut buf) {
        Ok(Some(n)) => n,
        Ok(None) => {
            return None;
        }
        Err(e) => {
            error!("nic.recv_nonblocking failed: {}", e);
            return None;
        }
    };
    let (layers, payload) = match parse_packet_with_ctx(&buf[..n], LayerId::Ether, &reg, &ctx) {
        Ok(v) => v,
        Err(e) => {
            error!("parse_packet_with_ctx failed: {}", e);
            return None;
        }
    };
    let ip = layers
        .iter()
        .find(|l| l.id == LayerId::Ipv4)
        .and_then(|l| l.downcast_ref::<Ipv4>());
    let udp = layers
        .iter()
        .find(|l| l.id == LayerId::Udp)
        .and_then(|l| l.downcast_ref::<Udp>());

    let (Some(ip), Some(udp)) = (ip, udp) else {
        return None;
    };

    let flow_key = socket::udp::Key {
        id: 0,
        peer_ip: ip.src,
        peer_port: udp.src_port,
        local_ip: ip.dst,
        local_port: udp.dst_port,
    };

    let sock_id = udp_sockets.peek(&flow_key).map(|c| c.key.id);

    Some((sock_id, payload.to_vec()))
}

// Extract a sock_id from already-parsed IPv4+UDP metadata.
// This keeps unit tests independent of AF_PACKET.
#[cfg(test)]
#[inline]
fn correlate_udp_sock_id(
    conns: &socket::udp::Table,
    ip: &Ipv4,
    udp: &Udp,
) -> Option<resources::SockId> {
    let flow_key = socket::udp::Key {
        id: 0,
        peer_ip: ip.src,
        peer_port: udp.src_port,
        local_ip: ip.dst,
        local_port: udp.dst_port,
    };
    conns.peek(&flow_key).map(|c| c.key.id)
}

/// Test helper: seed the shared UDP conn table with a synthetic connection entry and
/// return its sock_id.
#[cfg(test)]
#[allow(dead_code)]
fn seed_udp_conn_for_test(
    conns: &mut socket::udp::Table,
    peer_ip: ntx_network::Ipv4Addr,
    peer_port: u16,
    local_ip: ntx_network::Ipv4Addr,
    local_port: u16,
) -> resources::SockId {
    let conn = conns.connect(
        peer_ip,
        peer_port,
        local_ip,
        local_port,
        ntx_network::MacAddr([0, 1, 2, 3, 4, 5]),
        ntx_network::MacAddr([6, 7, 8, 9, 10, 11]),
        64,
    );
    conn.key.id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::logger_init;

    #[test]
    fn test_kernel_init() {
        logger_init();
        match Kernel::new() {
            Ok(_) => {}
            Err(e) => {
                // Running in CI / unprivileged user often can't open AF_PACKET.
                let is_permission_error = e.chain().any(|cause| {
                    let s = cause.to_string();
                    s.contains("Operation not permitted")
                        || s.contains("os error 1")
                        || s.contains("Permission denied")
                });

                if is_permission_error {
                    println!("Skipping test: requires privileges for AF_PACKET socket");
                    return;
                }

                panic!("Kernel::new failed: {e}");
            }
        }
    }

    #[test]
    fn request_resources_auto_acquire_and_pin_publishes_abr() {
        // No AF_PACKET required; this test only uses the resource pools logic.
        let mut cfg = ntx_network::resources::ResourcePoolsConfig::new();
        cfg.ipv4.push(ntx_network::resources::Ipv4PoolConfig {
            name: "default".to_string(),
            cidr: "10.0.0.0/30".to_string(),
            exclude: vec![],
        });
        cfg.mac.push(ntx_network::resources::MacPoolConfig {
            name: "default".to_string(),
            start: "02:00:00:00:00:10".to_string(),
            end: "02:00:00:00:00:10".to_string(),
            exclude: vec![],
        });
        cfg.udp_port.push(ntx_network::resources::PortPoolConfig {
            name: "default".to_string(),
            protocol: None,
            start: 10000,
            end: 10001,
            exclude: vec![],
        });
        cfg.tcp_port.push(ntx_network::resources::PortPoolConfig {
            name: "default".to_string(),
            protocol: None,
            start: 20000,
            end: 20001,
            exclude: vec![],
        });

        let mut pools = cfg.build().expect("build pools");
        let mut store = abr::BindingStore::default();

        let owner = uuid::Uuid::new_v4();

        let req = ResourceRequest {
            owner: Some(owner),
            ..ResourceRequest::default()
        };

        apply_resource_request_to(&mut pools, &mut store, &req).expect("apply request");

        let view = abr::load_view();
        // We should have at least one ipv4 and one udp/tcp port binding in the ABR view.
        assert!(!view.ipv4.is_empty());
        assert!(!view.udp_ports.is_empty());
        assert!(!view.tcp_ports.is_empty());
    }

    fn apply_resource_request_to(
        pools: &mut ResourcePools,
        store: &mut abr::BindingStore,
        req: &ResourceRequest,
    ) -> Result<()> {
        let owner = req.owner.unwrap_or_else(|| {
            pools.alloc_socket_owner(
                req.owner_name
                    .clone()
                    .unwrap_or_else(|| "socket".to_string()),
            )
        });

        let ipv4_pool = if req.ipv4_pool.is_empty() {
            "default"
        } else {
            req.ipv4_pool.as_str()
        };
        let udp_pool = if req.udp_port_pool.is_empty() {
            "default"
        } else {
            req.udp_port_pool.as_str()
        };
        let tcp_pool = if req.tcp_port_pool.is_empty() {
            "default"
        } else {
            req.tcp_port_pool.as_str()
        };
        let mac_pool = if req.mac_pool.is_empty() {
            "default"
        } else {
            req.mac_pool.as_str()
        };

        let wants_auto_acquire = req.pin_ipv4.is_none()
            && req.pin_mac.is_none()
            && req.pin_udp_ports.is_empty()
            && req.pin_tcp_ports.is_empty();

        if let Some(pool) = pools.ipv4(ipv4_pool) {
            if let Some(ip) = req.pin_ipv4 {
                pool.pin(owner, ntx_network::Ipv4Addr(ip.octets()))?;
            } else if wants_auto_acquire {
                let Some(ip) = pool.acquire_for(&owner) else {
                    anyhow::bail!("no available ipv4 in pool {ipv4_pool}");
                };
                pool.pin(owner, ip)?;
            }
        }

        // Note: this helper only exists to validate ABR publishing in isolation.
        // Keep MAC behavior minimal to avoid double-pin semantics across helpers.
        if let Some(mac) = req.pin_mac {
            if let Some(pool) = pools.mac(mac_pool) {
                pool.pin(owner, mac)?;
            }
        }

        let mac_pool = if req.mac_pool.is_empty() {
            "default"
        } else {
            req.mac_pool.as_str()
        };

        if let Some(pool) = pools.mac(mac_pool) {
            if let Some(mac) = req.pin_mac {
                pool.pin(owner, mac)?;
            } else if wants_auto_acquire {
                let Some(mac) = pool.acquire_for(&owner) else {
                    anyhow::bail!("no available mac in pool {mac_pool}");
                };
                pool.pin(owner, mac)?;
            }
        }

        if let Some(pool) = pools.udp_port(udp_pool) {
            if !req.pin_udp_ports.is_empty() {
                for p in &req.pin_udp_ports {
                    pool.pin(owner, *p)?;
                }
            } else if wants_auto_acquire {
                let Some(port) = pool.acquire_for(&owner) else {
                    anyhow::bail!("no available udp port in pool {udp_pool}");
                };
                pool.pin(owner, port)?;
            }
        }

        if let Some(pool) = pools.tcp_port(tcp_pool) {
            if !req.pin_tcp_ports.is_empty() {
                for p in &req.pin_tcp_ports {
                    pool.pin(owner, *p)?;
                }
            } else if wants_auto_acquire {
                let Some(port) = pool.acquire_for(&owner) else {
                    anyhow::bail!("no available tcp port in pool {tcp_pool}");
                };
                pool.pin(owner, port)?;
            }
        }

        pools.publish_abr_for_owner(store, &owner, abr::BindingOwner::KernelIface);
        Ok(())
    }

    #[test]
    fn udp_conn_table_correlates_sock_id_by_tuple() {
        // IMPORTANT: don't use `KERNEL` here.
        // `Kernel::new()` opens AF_PACKET and fails under unprivileged CI/user.
        let mut conns = socket::udp::Table::new(ConnTableConfig {
            max_entries: 4096,
            ttl: None,
        });

        // Seed one conn.
        let peer_ip = ntx_network::Ipv4Addr([10, 0, 0, 1]);
        let local_ip = ntx_network::Ipv4Addr([10, 0, 0, 2]);
        let peer_port = 1111;
        let local_port = 2222;
        let expected = seed_udp_conn_for_test(&mut conns, peer_ip, peer_port, local_ip, local_port);

        // Build minimal layer structs (no full packet required).
        let ip = Ipv4 {
            src: peer_ip,
            dst: local_ip,
            proto: 17,
            ttl: 64,
            identification: 0,
            flags_fragment: 0,
            ihl_bytes: 20,
        };
        let udp = Udp {
            src_port: peer_port,
            dst_port: local_port,
            src_ip: Some(peer_ip),
            dst_ip: Some(local_ip),
        };

        let got = correlate_udp_sock_id(&conns, &ip, &udp);
        assert_eq!(got, Some(expected));
    }
}
