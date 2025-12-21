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

    /// Allocate a fresh socket `owner_id` (ResourceId) and register it.
    ///
    /// This `owner_id` should be used as `resources::OwnerId` when allocating
    /// other resources (MAC/IP/ports).
    pub fn alloc_socket_owner(&mut self, name: impl Into<String>) -> ResourceId {
        let socket_id = self.registry.alloc_resource_id();
        self.registry
            .register_socket(socket_id, SocketInfo { name: name.into() });
        socket_id
    }

    /// Allocate a resource id and acquire+pin a UDP port for `owner`.
    ///
    /// Returns `(resource_id, port)`.
    pub fn alloc_udp_port_for(
        &mut self,
        pool_name: &str,
        owner: OwnerId,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<(ResourceId, u16)> {
        let rid = self.registry.alloc_resource_id();

        let Some(pool) = self.udp_port(pool_name) else {
            anyhow::bail!("missing udp_port pool: {pool_name}");
        };
        let Some(port) = pool.acquire_for(&owner) else {
            anyhow::bail!("no available udp port in pool {pool_name}");
        };

        pool.pin(owner, port)?;
        self.registry
            .register_non_socket(rid, ResourceKind::UdpPort, owner, using_sock_id);
        Ok((rid, port))
    }

    /// Allocate a resource id and acquire+pin an IPv4 address for `owner`.
    ///
    /// Returns `(resource_id, ipv4)`.
    pub fn alloc_ipv4_for(
        &mut self,
        pool_name: &str,
        owner: OwnerId,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<(ResourceId, Ipv4Addr)> {
        let rid = self.registry.alloc_resource_id();

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
        Ok((rid, addr))
    }

    /// Allocate a resource id and acquire+pin a MAC address for `owner`.
    ///
    /// Returns `(resource_id, mac)`.
    pub fn alloc_mac_for(
        &mut self,
        pool_name: &str,
        owner: OwnerId,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<(ResourceId, crate::MacAddr)> {
        let rid = self.registry.alloc_resource_id();

        let Some(pool) = self.mac(pool_name) else {
            anyhow::bail!("missing mac pool: {pool_name}");
        };
        let Some(mac) = pool.acquire_for(&owner) else {
            anyhow::bail!("no available mac in pool {pool_name}");
        };

        pool.pin(owner, mac)?;
        self.registry
            .register_non_socket(rid, ResourceKind::Mac, owner, using_sock_id);
        Ok((rid, mac))
    }

    /// Allocate a resource id and acquire+pin a TCP port for `owner`.
    ///
    /// Returns `(resource_id, port)`.
    pub fn alloc_tcp_port_for(
        &mut self,
        pool_name: &str,
        owner: OwnerId,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<(ResourceId, u16)> {
        let rid = self.registry.alloc_resource_id();

        let Some(pool) = self.tcp_port(pool_name) else {
            anyhow::bail!("missing tcp_port pool: {pool_name}");
        };
        let Some(port) = pool.acquire_for(&owner) else {
            anyhow::bail!("no available tcp port in pool {pool_name}");
        };

        pool.pin(owner, port)?;
        self.registry
            .register_non_socket(rid, ResourceKind::TcpPort, owner, using_sock_id);
        Ok((rid, port))
    }

    /// Pin an explicit UDP port for `owner` and return a new `resource_id`.
    pub fn pin_udp_port_with_id(
        &mut self,
        pool_name: &str,
        owner: OwnerId,
        port: u16,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<ResourceId> {
        let rid = self.registry.alloc_resource_id();
        let Some(pool) = self.udp_port(pool_name) else {
            anyhow::bail!("missing udp_port pool: {pool_name}");
        };
        pool.pin(owner, port)?;
        self.registry
            .register_non_socket(rid, ResourceKind::UdpPort, owner, using_sock_id);
        Ok(rid)
    }

    /// Pin an explicit TCP port for `owner` and return a new `resource_id`.
    pub fn pin_tcp_port_with_id(
        &mut self,
        pool_name: &str,
        owner: OwnerId,
        port: u16,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<ResourceId> {
        let rid = self.registry.alloc_resource_id();
        let Some(pool) = self.tcp_port(pool_name) else {
            anyhow::bail!("missing tcp_port pool: {pool_name}");
        };
        pool.pin(owner, port)?;
        self.registry
            .register_non_socket(rid, ResourceKind::TcpPort, owner, using_sock_id);
        Ok(rid)
    }

    /// Pin an explicit IPv4 address for `owner` and return a new `resource_id`.
    pub fn pin_ipv4_with_id(
        &mut self,
        pool_name: &str,
        owner: OwnerId,
        ipv4: Ipv4Addr,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<ResourceId> {
        let rid = self.registry.alloc_resource_id();
        let Some(pool) = self.ipv4(pool_name) else {
            anyhow::bail!("missing ipv4 pool: {pool_name}");
        };
        pool.pin(owner, crate::Ipv4Addr(ipv4.octets()))?;
        self.registry
            .register_non_socket(rid, ResourceKind::Ipv4, owner, using_sock_id);

        // Reverse index for kernel RX correlation: dst ip -> owner.
        self.registry.register_ipv4_owner(ipv4, owner);
        Ok(rid)
    }

    /// Pin an explicit MAC address for `owner` and return a new `resource_id`.
    pub fn pin_mac_with_id(
        &mut self,
        pool_name: &str,
        owner: OwnerId,
        mac: crate::MacAddr,
        using_sock_id: Option<SockId>,
    ) -> anyhow::Result<ResourceId> {
        let rid = self.registry.alloc_resource_id();
        let Some(pool) = self.mac(pool_name) else {
            anyhow::bail!("missing mac pool: {pool_name}");
        };
        pool.pin(owner, mac)?;
        self.registry
            .register_non_socket(rid, ResourceKind::Mac, owner, using_sock_id);
        Ok(rid)
    }

    /// Get a mutable IPv4 pool by name.
    #[inline]
    pub fn ipv4(&mut self, name: &str) -> Option<&mut Ipv4Pool> {
        self.ipv4.get_mut(name)
    }

    /// Get a mutable MAC pool by name.
    #[inline]
    pub fn mac(&mut self, name: &str) -> Option<&mut MacPool> {
        self.mac.get_mut(name)
    }

    /// Get a mutable port pool by name.
    #[inline]
    pub fn udp_port(&mut self, name: &str) -> Option<&mut PortPool> {
        self.udp_port.get_mut(name)
    }

    /// Get a mutable TCP port pool by name.
    #[inline]
    pub fn tcp_port(&mut self, name: &str) -> Option<&mut PortPool> {
        self.tcp_port.get_mut(name)
    }

    /// Back-compat alias: `port()` returns UDP pool.
    #[inline]
    pub fn port(&mut self, name: &str) -> Option<&mut PortPool> {
        self.udp_port(name)
    }
}
