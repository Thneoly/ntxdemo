use super::{Ipv4Pool, MacPool};
use super::{NamedPools, PortPool, ResourcePoolsConfig};

/// Logical owner id for pin/reserve operations (component id, task id, container id, ...).
pub type OwnerId = String;

/// Runtime pool set.
#[derive(Debug, Clone)]
pub struct ResourcePools {
    pub ipv4: NamedPools<Ipv4Pool>,
    pub mac: NamedPools<MacPool>,
    pub udp_port: NamedPools<PortPool>,
    pub tcp_port: NamedPools<PortPool>,
}

impl ResourcePools {
    pub fn empty() -> Self {
        Self {
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
            ipv4,
            mac,
            udp_port,
            tcp_port,
        })
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
