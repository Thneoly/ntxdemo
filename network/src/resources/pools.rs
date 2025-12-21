use super::{Ipv4Pool, MacPool};
use super::{NamedPools, PortPool, ResourcePoolsConfig};
use super::{ResourceId, ResourceKind, SockId};
use super::{ResourceRegistry, SocketInfo};
use std::net::Ipv4Addr;

use uuid::Uuid;

/// Logical owner id for pin/reserve operations (component id, task id, container id, ...).
pub type OwnerId = Uuid;

/// Runtime pool set.
#[derive(Debug, Clone)]
pub struct ResourcePools {
    pub registry: ResourceRegistry,
    pub ipv4: NamedPools<Ipv4Pool>,
    pub mac: NamedPools<MacPool>,
    pub udp_port: NamedPools<PortPool>,
    pub tcp_port: NamedPools<PortPool>,
}

/// Concrete value returned by generic resource acquisition APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonSocketResourceValue {
    Ipv4(Ipv4Addr),
    Mac(crate::MacAddr),
    UdpPort(u16),
    TcpPort(u16),
}

impl ResourcePools {
    pub fn new() -> Self {
        Self::empty()
    }
    pub fn empty() -> Self {
        Self {
            registry: ResourceRegistry::default(),
            ipv4: NamedPools::empty(),
            mac: NamedPools::empty(),
            udp_port: NamedPools::empty(),
            tcp_port: NamedPools::empty(),
        }
    }
    pub fn from_config(cfg: &ResourcePoolsConfig) -> anyhow::Result<Self> {
        let ipv4 = NamedPools::from_ipv4_configs(&cfg.ipv4)?;
        let mac = NamedPools::from_mac_configs(&cfg.mac)?;

        // Back-compat: treat legacy `port:` as UDP.
        let mut udp_cfgs = cfg.udp_port.clone();
        udp_cfgs.extend(cfg.port.iter().cloned());
        let udp_port = NamedPools::from_port_configs(&udp_cfgs)?;

        let tcp_port = NamedPools::from_port_configs(&cfg.tcp_port)?;

        Ok(Self {
            registry: ResourceRegistry::default(),
            ipv4,
            mac,
            udp_port,
            tcp_port,
        })
    }

    #[inline]
    pub fn registry(&self) -> &ResourceRegistry {
        &self.registry
    }

    #[inline]
    pub fn registry_mut(&mut self) -> &mut ResourceRegistry {
        &mut self.registry
    }

    /// Acquire a fresh socket `owner_id` (ResourceId) and register it.
    ///
    /// This `owner_id` should be used as `resources::OwnerId` when acquiring
    /// other resources (MAC/IP/ports).
    pub fn acquire_socket_owner(&mut self, name: impl Into<String>) -> ResourceId {
        let socket_id = self.registry.acquire_resource_id();
        self.registry.register_socket(
            socket_id,
            SocketInfo {
                name: name.into(),
                sock_id: None,
            },
        );
        socket_id
    }

    #[inline]
    fn sock_id_for_owner(&self, owner: &OwnerId) -> Option<SockId> {
        self.registry.socket_info(owner).and_then(|s| s.sock_id)
    }

    // Note: socket-specific acquire_* wrappers were intentionally removed to keep a single
    // acquisition entrypoint for non-socket resources; callers should infer `using_sock_id`
    // (if desired) via the registry and pass it into `acquire_and_pin_non_socket`.

    /// Generic (public) API: acquire a new `ResourceId`, acquire from pool, pin it, and
    /// register it in the registry as a non-socket resource.
    ///
    /// This is the preferred external entrypoint for non-socket acquisitions.
    pub fn acquire_and_pin_non_socket(
        &mut self,
        kind: ResourceKind,
        pool_name: &str,
        owner: OwnerId,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<(ResourceId, NonSocketResourceValue)> {
        let rid = self.registry.acquire_resource_id();

        let value = match kind {
            ResourceKind::Ipv4 => {
                let Some(pool) = self.ipv4(pool_name) else {
                    anyhow::bail!("missing ipv4 pool: {pool_name}");
                };
                let Some(ip) = pool.acquire_for(&owner) else {
                    anyhow::bail!("no available ipv4 in pool {pool_name}");
                };
                let addr = Ipv4Addr::from(ip.0);
                pool.pin(owner, ip)?;
                self.registry
                    .register_non_socket(rid, ResourceKind::Ipv4, owner, using_sock_id);
                // Reverse index for kernel RX correlation: dst ip -> owner.
                self.registry.register_ipv4_owner(addr, owner);
                NonSocketResourceValue::Ipv4(addr)
            }
            ResourceKind::Mac => {
                let Some(pool) = self.mac(pool_name) else {
                    anyhow::bail!("missing mac pool: {pool_name}");
                };
                let Some(mac) = pool.acquire_for(&owner) else {
                    anyhow::bail!("no available mac in pool {pool_name}");
                };
                pool.pin(owner, mac)?;
                self.registry
                    .register_non_socket(rid, ResourceKind::Mac, owner, using_sock_id);
                NonSocketResourceValue::Mac(mac)
            }
            ResourceKind::UdpPort => {
                let Some(pool) = self.udp_port(pool_name) else {
                    anyhow::bail!("missing udp_port pool: {pool_name}");
                };
                let Some(port) = pool.acquire_for(&owner) else {
                    anyhow::bail!("no available udp port in pool {pool_name}");
                };
                pool.pin(owner, port)?;
                self.registry
                    .register_non_socket(rid, ResourceKind::UdpPort, owner, using_sock_id);
                NonSocketResourceValue::UdpPort(port)
            }
            ResourceKind::TcpPort => {
                let Some(pool) = self.tcp_port(pool_name) else {
                    anyhow::bail!("missing tcp_port pool: {pool_name}");
                };
                let Some(port) = pool.acquire_for(&owner) else {
                    anyhow::bail!("no available tcp port in pool {pool_name}");
                };
                pool.pin(owner, port)?;
                self.registry
                    .register_non_socket(rid, ResourceKind::TcpPort, owner, using_sock_id);
                NonSocketResourceValue::TcpPort(port)
            }
            ResourceKind::Socket => {
                anyhow::bail!("acquire_and_pin_non_socket does not support ResourceKind::Socket")
            }
        };

        Ok((rid, value))
    }

    /// Generic (public) API: pin an explicit concrete value and return a new `ResourceId`.
    ///
    /// This is the preferred external entrypoint for pinning non-socket resources.
    pub fn pin_non_socket_with_id(
        &mut self,
        kind: ResourceKind,
        pool_name: &str,
        owner: OwnerId,
        value: NonSocketResourceValue,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<ResourceId> {
        let rid = self.registry.acquire_resource_id();

        match (kind, value) {
            (ResourceKind::Ipv4, NonSocketResourceValue::Ipv4(ipv4)) => {
                let Some(pool) = self.ipv4(pool_name) else {
                    anyhow::bail!("missing ipv4 pool: {pool_name}");
                };
                pool.pin(owner, crate::Ipv4Addr(ipv4.octets()))?;
                self.registry
                    .register_non_socket(rid, ResourceKind::Ipv4, owner, using_sock_id);
                // Reverse index for kernel RX correlation: dst ip -> owner.
                self.registry.register_ipv4_owner(ipv4, owner);
            }
            (ResourceKind::Mac, NonSocketResourceValue::Mac(mac)) => {
                let Some(pool) = self.mac(pool_name) else {
                    anyhow::bail!("missing mac pool: {pool_name}");
                };
                pool.pin(owner, mac)?;
                self.registry
                    .register_non_socket(rid, ResourceKind::Mac, owner, using_sock_id);
            }
            (ResourceKind::UdpPort, NonSocketResourceValue::UdpPort(port)) => {
                let Some(pool) = self.udp_port(pool_name) else {
                    anyhow::bail!("missing udp_port pool: {pool_name}");
                };
                pool.pin(owner, port)?;
                self.registry
                    .register_non_socket(rid, ResourceKind::UdpPort, owner, using_sock_id);
            }
            (ResourceKind::TcpPort, NonSocketResourceValue::TcpPort(port)) => {
                let Some(pool) = self.tcp_port(pool_name) else {
                    anyhow::bail!("missing tcp_port pool: {pool_name}");
                };
                pool.pin(owner, port)?;
                self.registry
                    .register_non_socket(rid, ResourceKind::TcpPort, owner, using_sock_id);
            }
            (ResourceKind::Socket, _) => {
                anyhow::bail!("pin_non_socket_with_id does not support ResourceKind::Socket")
            }
            _ => {
                anyhow::bail!("resource kind/value mismatch")
            }
        }

        Ok(rid)
    }

    /// Get a mutable IPv4 pool by name.
    #[inline]
    pub(crate) fn ipv4(&mut self, name: &str) -> Option<&mut Ipv4Pool> {
        self.ipv4.get_mut(name)
    }

    /// Get a mutable MAC pool by name.
    #[inline]
    pub(crate) fn mac(&mut self, name: &str) -> Option<&mut MacPool> {
        self.mac.get_mut(name)
    }

    /// Get a mutable port pool by name.
    #[inline]
    pub(crate) fn udp_port(&mut self, name: &str) -> Option<&mut PortPool> {
        self.udp_port.get_mut(name)
    }

    /// Get a mutable TCP port pool by name.
    #[inline]
    pub(crate) fn tcp_port(&mut self, name: &str) -> Option<&mut PortPool> {
        self.tcp_port.get_mut(name)
    }

    /// Back-compat alias: `port()` returns UDP pool.
    #[inline]
    pub(crate) fn port(&mut self, name: &str) -> Option<&mut PortPool> {
        self.udp_port(name)
    }

    /// Unified resolver for non-socket resources.
    ///
    /// This is the preferred external entrypoint when the caller knows the `ResourceKind`
    /// and wants the concrete pinned value.
    ///
    /// Notes:
    /// - Resolution is `resource_id -> owner_id -> owner_to_pinned[...]`.
    /// - For port pools, an owner can have multiple pinned ports; this returns the first.
    pub fn resolve_non_socket(
        &self,
        kind: ResourceKind,
        rid: &ResourceId,
    ) -> Option<NonSocketResourceValue> {
        match kind {
            ResourceKind::Ipv4 => {
                let owner = self.registry.owner_of(rid)?;
                let pool = self
                    .ipv4
                    .inner
                    .values()
                    .find(|p| p.owner_to_pinned.contains_key(&owner))?;
                pool.owner_to_pinned
                    .get(&owner)
                    .map(|ip| NonSocketResourceValue::Ipv4(std::net::Ipv4Addr::from(ip.0)))
            }
            ResourceKind::Mac => {
                let owner = self.registry.owner_of(rid)?;
                let pool = self
                    .mac
                    .inner
                    .values()
                    .find(|p| p.owner_to_pinned.contains_key(&owner))?;
                pool.owner_to_pinned
                    .get(&owner)
                    .copied()
                    .map(NonSocketResourceValue::Mac)
            }
            ResourceKind::UdpPort => {
                let owner = self.registry.owner_of(rid)?;
                let pool = self
                    .udp_port
                    .inner
                    .values()
                    .find(|p| p.owner_to_pinned.contains_key(&owner))?;
                pool.owner_to_pinned
                    .get(&owner)
                    .and_then(|ports| ports.iter().next().copied())
                    .map(NonSocketResourceValue::UdpPort)
            }
            ResourceKind::TcpPort => {
                let owner = self.registry.owner_of(rid)?;
                let pool = self
                    .tcp_port
                    .inner
                    .values()
                    .find(|p| p.owner_to_pinned.contains_key(&owner))?;
                pool.owner_to_pinned
                    .get(&owner)
                    .and_then(|ports| ports.iter().next().copied())
                    .map(NonSocketResourceValue::TcpPort)
            }
            ResourceKind::Socket => None,
        }
    }
}
