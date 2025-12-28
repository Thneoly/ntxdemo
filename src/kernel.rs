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

use ntx_network::resources::NonSocketResourceValue;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Deserialize;
use std::path::Path;
use tracing::{error, info};

/// Publish a fresh ABR snapshot that includes resources for *all* currently-registered
/// socket owners.
///
/// Why this exists:
/// `ResourcePools::publish_abr_for_owner` clears the `BindingStore` and then publishes
/// bindings for a *single* owner. In a multi-socket/multi-owner runtime, doing that on
/// every acquire/bind causes "last writer wins" behavior and can make
/// `network::packet::layers::Ipv4::accept()` return `Poison` for other owners' dst_ip.
///
/// Implementation detail:
/// We intentionally build the union view using only **public APIs** (no peeking into
/// pool internals). We iterate the owners tracked by the kernel's registry list.
fn publish_kernel_abr_all_owners(pools: &ResourcePools, store: &mut abr::BindingStore) {
    pools.publish_abr_for_all_owners(store, abr::BindingOwner::KernelIface);

    // Debugging aid: confirm ABR contents after publishing.
    // This is intentionally cheap (snapshot is already stored by publish call).
    if tracing::enabled!(tracing::Level::DEBUG) {
        let view = abr::load_view();
        // Common lab IP, convenient for logs; harmless if not used.
        let ip_10_0_0_4_be = u32::from_be_bytes([10, 0, 0, 4]);
        tracing::debug!(
            target: "ntx::kernel::abr",
            ipv4_empty = view.ipv4.is_empty(),
            udp_ports_empty = view.udp_ports.is_empty(),
            tcp_ports_empty = view.tcp_ports.is_empty(),
            has_10_0_0_4 = view.ipv4.contains_be(ip_10_0_0_4_be),
            "abr published (union)"
        );
    }
}

/// Minimal error type for hostnet/WIT adapters.
///
/// This lives in `kernel` so `wasm_engine` can delegate WIT host calls here,
/// and `kernel` can remain the single owner of control-plane state.
#[derive(thiserror::Error, Debug)]
pub enum HostnetError {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("no space")]
    NoSpace,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Frame handle used by hostnet build-reply path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostnetFrameHandle {
    pub region: u32,
    pub offset: u32,
    pub len: u32,
}

/// Result of hostnet UDP create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostnetUdpSocket {
    pub owner: String,
    pub sock_id: u64,
}

fn parse_uuid_str(s: &str) -> Result<uuid::Uuid, HostnetError> {
    uuid::Uuid::parse_str(s)
        .map_err(|e| HostnetError::InvalidArgument(format!("invalid uuid `{s}`: {e}")))
}

fn fmt_uuid(id: &uuid::Uuid) -> String {
    id.to_string()
}

/// Allocate a new socket owner id and register it (hostnet WIT adapter).
pub fn hostnet_create_socket_owner(name: &str) -> Result<String, HostnetError> {
    let mut pools = KERNEL.pools.lock();
    let owner = pools.acquire_socket_owner(name.to_string());
    Ok(fmt_uuid(&owner))
}

/// Allocate+pin a UDP resources for this owner from pool (hostnet WIT adapter).
pub fn hostnet_acquire_udp_resource(pool: &str, owner: &str) -> Result<(), HostnetError> {
    let owner = parse_uuid_str(owner)?;
    let mut pools = KERNEL.pools.lock();
    let (_, v) = pools.acquire_and_pin_non_socket(resources::ResourceKind::Ipv4, pool, owner)?;
    let NonSocketResourceValue::Ipv4(_ip) = v else {
        unreachable!("resource kind/value mismatch")
    };
    info!(
        target: "ntx::kernel",
        "acquired ipv4 resource: {:?}",
        v
    );
    let (_, v) = pools.acquire_and_pin_non_socket(resources::ResourceKind::Mac, pool, owner)?;
    let NonSocketResourceValue::Mac(_mac) = v else {
        unreachable!("resource kind/value mismatch")
    };
    info!(
        target: "ntx::kernel",
        "acquired mac resource: {:?}",
        v
    );

    let (_, v) = pools.acquire_and_pin_non_socket(resources::ResourceKind::UdpPort, pool, owner)?;
    let NonSocketResourceValue::UdpPort(_port) = v else {
        unreachable!("resource kind/value mismatch")
    };

    // Publish updated ABR snapshot including all owners.
    let mut store = KERNEL.store.lock();
    publish_kernel_abr_all_owners(&pools, &mut store);
    Ok(())
}

/// Allocate+pin a UDP identity (local ip/mac/udp-port) for this owner from the given pool.
pub fn hostnet_acquire_udp_identity(
    pool: &str,
    owner: &str,
) -> Result<(std::net::Ipv4Addr, ntx_network::MacAddr, u16), HostnetError> {
    let owner = parse_uuid_str(owner)?;
    let mut pools = KERNEL.pools.lock();

    // Pool arg here is treated as the canonical pool name for this identity.
    // Use it for IPv4/MAC/UDP port for simplicity.
    let (_, v) = pools.acquire_and_pin_non_socket(resources::ResourceKind::Ipv4, pool, owner)?;
    let NonSocketResourceValue::Ipv4(ip) = v else {
        unreachable!("resource kind/value mismatch")
    };

    let (_, v) = pools.acquire_and_pin_non_socket(resources::ResourceKind::Mac, pool, owner)?;
    let NonSocketResourceValue::Mac(mac) = v else {
        unreachable!("resource kind/value mismatch")
    };

    let (_, v) = pools.acquire_and_pin_non_socket(resources::ResourceKind::UdpPort, pool, owner)?;
    let NonSocketResourceValue::UdpPort(port) = v else {
        unreachable!("resource kind/value mismatch")
    };

    // Publish updated ABR snapshot including all owners.
    // This is critical because `Ipv4::accept()` will Poison packets whose dst_ip isn't in ABR.
    let mut store = KERNEL.store.lock();
    publish_kernel_abr_all_owners(&pools, &mut store);

    Ok((ip, mac, port))
}
pub fn hostnet_resolve_udp_port(rid: &str) -> Result<u16, HostnetError> {
    let rid = parse_uuid_str(rid)?;
    let pools = KERNEL.pools.lock();
    pools
        .resolve_non_socket(resources::ResourceKind::UdpPort, &rid)
        .and_then(|v| match v {
            NonSocketResourceValue::UdpPort(p) => Some(p),
            _ => None,
        })
        .ok_or(HostnetError::NotFound(format!("udp port not found: {rid}")))
}

// ---- UDP socket control wrappers (binder + table live in kernel) ----

/// Create a UDP socket (owner + sock_id) using the kernel's conn table.
///
/// Note: this mirrors the WIT `udp-socket-control.create` semantics.
pub fn hostnet_udp_create(name: &str) -> Result<HostnetUdpSocket, HostnetError> {
    let mut pools = KERNEL.pools.lock();
    let table = KERNEL.udp_sockets.lock();
    let (owner, sock_id) = udp::create_udp_socket(&mut pools, &table, name.to_string());
    Ok(HostnetUdpSocket {
        owner: fmt_uuid(&owner),
        sock_id,
    })
}
fn hostnet_udp_bind_peer(
    sock: u64,
    peer_ipv4: ntx_network::Ipv4Addr,
    peer_port: u16,
    peer_mac: ntx_network::MacAddr,
) -> Result<(), HostnetError> {
    KERNEL
        .hostnet_udp_binder
        .lock()
        .bind_peer(sock, peer_ipv4, peer_port, peer_mac);
    Ok(())
}

fn hostnet_udp_bind_ttl(sock: u64, ttl: u8) -> Result<(), HostnetError> {
    KERNEL.hostnet_udp_binder.lock().bind_ttl(sock, ttl);
    Ok(())
}

/// Bind all required parameters in a single call.
///
/// This mirrors the WIT `udp-socket-control.bind` convenience API.
/// acquire: provides "identity" (who I am: local ip/mac/port, and owned by me)
/// bind: provides "routing entry" (which packets sent to me are mine; who I want to send to)
pub fn hostnet_udp_bind_all(
    sock: u64,
    local_ipv4: ntx_network::Ipv4Addr,
    local_mac: ntx_network::MacAddr,
    local_udp_port: u16,
    peer_ipv4: ntx_network::Ipv4Addr,
    peer_port: u16,
    peer_mac: ntx_network::MacAddr,
    ttl: Option<u8>,
) -> Result<(), HostnetError> {
    let pools = KERNEL.pools.lock();
    // Resolve the socket owner from sock_id, then use (owner + value) -> rid.
    let owner = pools
        .registry()
        .socket_id_for_sock_id(sock)
        .ok_or(HostnetError::NotFound(format!(
            "socket owner not found: {sock}"
        )))?;

    // Defensive: this API is value-based, so we must ensure the local resources we resolve
    // are owned by *this* socket owner (to prevent cross-socket hijacking).
    let ensure_owned_by_sock =
        |value: NonSocketResourceValue| -> Result<resources::ResourceId, HostnetError> {
            let rid =
                pools
                    .rid_for_non_socket_value(&owner, &value)
                    .ok_or(HostnetError::NotFound(format!(
                        "resource not found: {:?}",
                        value
                    )))?;
            match pools.registry().owner_of(&rid) {
                Some(o) if o == owner => Ok(rid),
                Some(_) => Err(HostnetError::InvalidArgument(
                    "local resource is not owned by this socket".to_string(),
                )),
                None => Err(HostnetError::NotFound(format!(
                    "resource owner not found: {rid}"
                ))),
            }
        };

    let local_ipv4_rid = ensure_owned_by_sock(NonSocketResourceValue::Ipv4(
        std::net::Ipv4Addr::from(local_ipv4.0),
    ))?;
    let local_mac_rid = ensure_owned_by_sock(NonSocketResourceValue::Mac(local_mac))?;
    let local_udp_port_rid = ensure_owned_by_sock(NonSocketResourceValue::UdpPort(local_udp_port))?;
    drop(pools);

    KERNEL
        .hostnet_udp_binder
        .lock()
        .bind_local_ipv4_rid(sock, local_ipv4_rid);
    KERNEL
        .hostnet_udp_binder
        .lock()
        .bind_local_mac_rid(sock, local_mac_rid);
    KERNEL
        .hostnet_udp_binder
        .lock()
        .bind_local_udp_port_rid(sock, local_udp_port_rid);
    hostnet_udp_bind_peer(sock, peer_ipv4, peer_port, peer_mac)?;
    if let Some(ttl) = ttl {
        hostnet_udp_bind_ttl(sock, ttl)?;
    }
    // One-step API: commit bindings into the conn-table.
    hostnet_udp_finalize(sock)?;

    Ok(())
}

pub fn hostnet_udp_finalize(sock: u64) -> Result<(), HostnetError> {
    let pools = KERNEL.pools.lock();
    let mut table = KERNEL.udp_sockets.lock();
    KERNEL
        .hostnet_udp_binder
        .lock()
        .finalize_into_table(&pools, &mut table, sock)
        .map_err(anyhow::Error::from)?;
    Ok(())
}

pub fn hostnet_udp_build_reply(
    sock: u64,
    payload: &[u8],
) -> Result<HostnetFrameHandle, HostnetError> {
    let mut table = KERNEL.udp_sockets.lock();
    let frame = table
        .build_reply_for_sock_id(sock, payload)
        .map_err(anyhow::Error::from)?;

    let mut arena = KERNEL.hostnet_tx_arena.lock();
    let mut head = KERNEL.hostnet_tx_head.lock();

    if frame.bytes.len() > arena.len() {
        return Err(HostnetError::NoSpace);
    }
    if *head + frame.bytes.len() > arena.len() {
        *head = 0;
    }
    let off = *head;
    arena[off..off + frame.bytes.len()].copy_from_slice(&frame.bytes);
    *head += frame.bytes.len();

    Ok(HostnetFrameHandle {
        region: 2,
        offset: off as u32,
        len: frame.bytes.len() as u32,
    })
}

/// Transmit a frame previously written into the hostnet TX arena.
///
/// Currently `hostnet_udp_build_reply` writes into (region=2) backed by
/// `KERNEL.hostnet_tx_arena`.
pub fn hostnet_tx(frame: HostnetFrameHandle) -> Result<u32, HostnetError> {
    // Validate handle.
    if frame.region != 2 {
        return Err(HostnetError::InvalidArgument(
            "unsupported frame region".to_string(),
        ));
    }

    let arena = KERNEL.hostnet_tx_arena.lock();
    let off = frame.offset as usize;
    let len = frame.len as usize;
    let end = off.saturating_add(len);
    if end > arena.len() {
        return Err(HostnetError::InvalidArgument(
            "frame range out of bounds".to_string(),
        ));
    }

    tracing::info!(
        target: "ntx::kernel",
        region = frame.region,
        offset = frame.offset,
        len = frame.len,
        "hostnet_tx: requested"
    );
    let bytes = &arena[off..end];
    let nic = KERNEL.nic();
    nic.send(bytes)
        .map_err(|e| HostnetError::Other(anyhow::Error::from(e)))?;
    tracing::info!(target: "ntx::kernel", sent_len = frame.len, "hostnet_tx: NIC send ok");
    Ok(frame.len)
}

/// Resource request options.
///
/// Kernel doesn't currently distinguish client/server; this interface is intended as a
/// control-plane hook to register/acquire resources and then refresh the ABR snapshot.
#[derive(Debug, Clone, Default)]
pub struct ResourceRequest {
    /// Socket owner id. If `None`, kernel will acquire a new socket owner id.
    ///
    /// This is a `resources::ResourceId` (UUID v4) and is used as `resources::OwnerId`
    /// for all subsequent resource acquisitions.
    pub owner: Option<resources::OwnerId>,

    /// Optional friendly name to store in the registry when `owner` is auto-acquired.
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

/// Resource request result (all acquired/pinned ids).
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
    #[allow(dead_code)]
    config: Config,
    reg: LayerRegistry,
    pools: Mutex<ResourcePools>,
    #[allow(dead_code)]
    store: Mutex<abr::BindingStore>,
    arp_cache: Mutex<ArpCache>,
    udp_sockets: Mutex<Table>,
    nic: Mutex<Box<dyn Nic + Send + Sync>>,

    // ---- hostnet (WIT adapter) control-plane state ----
    /// Binder state for WIT `udp-socket-control.*` calls.
    hostnet_udp_binder: Mutex<UdpSocketBinder>,
    /// 1 MiB MVP arena for build-reply outputs.
    hostnet_tx_arena: Mutex<Vec<u8>>,
    /// Current write head into `hostnet_tx_arena`.
    hostnet_tx_head: Mutex<usize>,
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
        let arp_cache = Mutex::new(ArpCache::new(std::time::Duration::from_secs(60)));
        let udp_sockets = Table::new(ConnTableConfig {
            max_entries: 4096,
            ttl: None,
        });
        let resource_pools_config = ResourcePoolsConfig::load_yaml_file(&cfg.resource.path)?;
        let pools = Mutex::new(resource_pools_config.build()?);
        let abr_store = Mutex::new(abr::BindingStore::default());

        let hostnet_udp_binder = Mutex::new(UdpSocketBinder::new());
        let hostnet_tx_arena = Mutex::new(vec![0u8; 1024 * 1024]);
        let hostnet_tx_head = Mutex::new(0usize);

        Ok(Self {
            config: cfg,
            reg,
            pools,
            store: abr_store,
            arp_cache,
            udp_sockets: Mutex::new(udp_sockets),
            nic,

            hostnet_udp_binder,
            hostnet_tx_arena,
            hostnet_tx_head,
        })
    }
    fn nic(&self) -> parking_lot::MutexGuard<'_, Box<dyn Nic + Send + Sync>> {
        self.nic.lock()
    }
    fn reg(&self) -> &LayerRegistry {
        &self.reg
    }
    fn udp_sockets(&self) -> parking_lot::MutexGuard<'_, Table> {
        self.udp_sockets.lock()
    }
}

/// Best-effort resolve peer MAC from IPv4 using kernel ARP cache.
pub fn hostnet_resolve_peer_mac(
    peer_ipv4: ntx_network::Ipv4Addr,
) -> Result<ntx_network::MacAddr, HostnetError> {
    let mut arp = KERNEL.arp_cache.lock();
    arp.get(peer_ipv4).ok_or_else(|| {
        HostnetError::NotFound(format!(
            "arp cache miss for peer ipv4: {}.{}.{}.{}",
            peer_ipv4.0[0], peer_ipv4.0[1], peer_ipv4.0[2], peer_ipv4.0[3]
        ))
    })
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

/// Refresh the cached ABR view from the global ABR snapshot.
///
/// NOTE: Scheme-B: kernel dataplane reads ABR directly via `abr::load_view()`.
/// This function is intentionally removed to avoid stale caching semantics.

/// UDP RX result with best-effort flow correlation.
#[derive(Debug, Clone)]
pub struct UdpRx {
    pub sock_id: Option<resources::SockId>,
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: Vec<u8>,
}

/// Receive one UDP datagram (non-blocking) and return dst/src ports + best-effort sock_id.
pub fn non_blocking_recv_udp() -> Option<UdpRx> {
    let mut nic = KERNEL.nic();
    let reg = KERNEL.reg();
    // Scheme-B: load ABR snapshot directly (RCU/ArcSwap atomic load).
    // Recommended pattern: load once per RX loop tick and reuse for parsing this packet.
    let abr_view = abr::load_view();
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
            error!(target: "ntx::kernel::rx", error = %e, frame_len = n, "parse_packet_with_ctx failed");
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

    let Some(ip) = ip else {
        tracing::warn!(target: "ntx::kernel::rx", frame_len = n, "non-IPv4 packet received; dropping");
        return None;
    };

    let Some(udp) = udp else {
        tracing::warn!(target: "ntx::kernel::rx", src_ip = ?ip.src, dst_ip = ?ip.dst, "non-UDP packet received; dropping");
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

    tracing::debug!(
        target: "ntx::kernel::rx",
        src_ip = ?ip.src,
        dst_ip = ?ip.dst,
        src_port = udp.src_port,
        dst_port = udp.dst_port,
        payload_len = payload.len(),
        sock_id = ?sock_id,
        "udp rx parsed"
    );

    Some(UdpRx {
        sock_id,
        src_port: udp.src_port,
        dst_port: udp.dst_port,
        payload: payload.to_vec(),
    })
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
            pools.acquire_socket_owner(
                req.owner_name
                    .clone()
                    .unwrap_or_else(|| "socket".to_string()),
            )
        });

        // Important for union-ABR publishing: ensure the owner exists as a registered socket.
        // In production, owners are created via `acquire_socket_owner`, which registers them.
        // This test helper also allows callers to pass `req.owner`, so we must register it.
        if pools.registry().socket_info(&owner).is_none() {
            pools.registry_mut().register_socket(
                owner,
                ntx_network::resources::SocketInfo {
                    name: req
                        .owner_name
                        .clone()
                        .unwrap_or_else(|| "socket".to_string()),
                    sock_id: None,
                },
            );
        }

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

        // Pin or auto-acquire ipv4.
        if let Some(ip) = req.pin_ipv4 {
            pools.pin_non_socket_with_id(
                resources::ResourceKind::Ipv4,
                ipv4_pool,
                owner,
                NonSocketResourceValue::Ipv4(ip),
            )?;
        } else if wants_auto_acquire {
            let (_rid, v) = pools.acquire_and_pin_non_socket(
                resources::ResourceKind::Ipv4,
                ipv4_pool,
                owner,
            )?;
            let NonSocketResourceValue::Ipv4(_ip) = v else {
                unreachable!("resource kind/value mismatch")
            };
        }

        // Pin or auto-acquire mac.
        if let Some(mac) = req.pin_mac {
            pools.pin_non_socket_with_id(
                resources::ResourceKind::Mac,
                mac_pool,
                owner,
                NonSocketResourceValue::Mac(mac),
            )?;
        } else if wants_auto_acquire {
            let (_rid, v) =
                pools.acquire_and_pin_non_socket(resources::ResourceKind::Mac, mac_pool, owner)?;
            let NonSocketResourceValue::Mac(_mac) = v else {
                unreachable!("resource kind/value mismatch")
            };
        }

        // Pin or auto-acquire udp ports.
        if !req.pin_udp_ports.is_empty() {
            for p in &req.pin_udp_ports {
                pools.pin_non_socket_with_id(
                    resources::ResourceKind::UdpPort,
                    udp_pool,
                    owner,
                    NonSocketResourceValue::UdpPort(*p),
                )?;
            }
        } else if wants_auto_acquire {
            let (_rid, v) = pools.acquire_and_pin_non_socket(
                resources::ResourceKind::UdpPort,
                udp_pool,
                owner,
            )?;
            let NonSocketResourceValue::UdpPort(_p) = v else {
                unreachable!("resource kind/value mismatch")
            };
        }

        // Pin or auto-acquire tcp ports.
        if !req.pin_tcp_ports.is_empty() {
            for p in &req.pin_tcp_ports {
                pools.pin_non_socket_with_id(
                    resources::ResourceKind::TcpPort,
                    tcp_pool,
                    owner,
                    NonSocketResourceValue::TcpPort(*p),
                )?;
            }
        } else if wants_auto_acquire {
            let (_rid, v) = pools.acquire_and_pin_non_socket(
                resources::ResourceKind::TcpPort,
                tcp_pool,
                owner,
            )?;
            let NonSocketResourceValue::TcpPort(_p) = v else {
                unreachable!("resource kind/value mismatch")
            };
        }

        pools.publish_abr_for_all_owners(store, abr::BindingOwner::KernelIface);
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
