use std::collections::BTreeMap;

use anyhow::Result;

use super::{Ipv4Pool, Ipv4PoolConfig, MacPool, MacPoolConfig, PortPool, PortPoolConfig};

/// A set of pools keyed by name.
///
/// Convention:
/// - if a config entry doesn't specify `name`, it is grouped under "default".
#[derive(Debug, Clone, Default)]
pub struct NamedPools<T> {
    pub(crate) inner: BTreeMap<String, T>,
}

impl<T> NamedPools<T> {
    #[inline]
    pub fn empty() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }

    #[inline]
    pub fn get_mut(&mut self, name: &str) -> Option<&mut T> {
        self.inner.get_mut(name)
    }

    #[inline]
    pub fn get(&self, name: &str) -> Option<&T> {
        self.inner.get(name)
    }

    #[inline]
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.inner.keys().map(|s| s.as_str())
    }
}

impl NamedPools<Ipv4Pool> {
    pub(crate) fn from_ipv4_configs(cfgs: &[Ipv4PoolConfig]) -> Result<Self> {
        let mut groups: BTreeMap<String, Vec<Ipv4PoolConfig>> = BTreeMap::new();
        for cfg in cfgs {
            let name = if cfg.name.is_empty() {
                "default".to_string()
            } else {
                cfg.name.clone()
            };
            groups.entry(name).or_default().push(cfg.clone());
        }

        let mut inner = BTreeMap::new();
        for (name, group_cfgs) in groups {
            inner.insert(name, Ipv4Pool::from_config(&group_cfgs)?);
        }
        Ok(Self { inner })
    }
}

impl NamedPools<MacPool> {
    pub(crate) fn from_mac_configs(cfgs: &[MacPoolConfig]) -> Result<Self> {
        let mut groups: BTreeMap<String, Vec<MacPoolConfig>> = BTreeMap::new();
        for cfg in cfgs {
            let name = if cfg.name.is_empty() {
                "default".to_string()
            } else {
                cfg.name.clone()
            };
            groups.entry(name).or_default().push(cfg.clone());
        }

        let mut inner = BTreeMap::new();
        for (name, group_cfgs) in groups {
            inner.insert(name, MacPool::from_config(&group_cfgs)?);
        }
        Ok(Self { inner })
    }
}

impl NamedPools<PortPool> {
    pub(crate) fn from_port_configs(cfgs: &[PortPoolConfig]) -> Result<Self> {
        let mut groups: BTreeMap<String, Vec<PortPoolConfig>> = BTreeMap::new();
        for cfg in cfgs {
            let name = if cfg.name.is_empty() {
                "default".to_string()
            } else {
                cfg.name.clone()
            };
            groups.entry(name).or_default().push(cfg.clone());
        }

        let mut inner = BTreeMap::new();
        for (name, group_cfgs) in groups {
            inner.insert(name, PortPool::from_config(&group_cfgs)?);
        }
        Ok(Self { inner })
    }
}
