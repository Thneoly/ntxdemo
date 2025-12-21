use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{Ipv4PoolConfig, MacPoolConfig, PortPoolConfig, ResourcePools};

/// Protocol selector for port pools.
///
/// Note: current YAML schema prefers top-level `udp_port:` and `tcp_port:` lists.
/// This enum exists for future-proofing and potential in-entry overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Udp,
    Tcp,
    Raw,
    Ethernet,
}

/// Top-level config schema.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ResourcePoolsConfig {
    #[serde(default)]
    pub mac: Vec<MacPoolConfig>,
    #[serde(default)]
    pub ipv4: Vec<Ipv4PoolConfig>,

    /// Legacy/compat: treated as UDP port pools.
    ///
    /// Prefer using `udp_port` and/or `tcp_port`.
    #[serde(default)]
    pub port: Vec<PortPoolConfig>,

    /// UDP port pools.
    #[serde(default)]
    pub udp_port: Vec<PortPoolConfig>,

    /// TCP port pools.
    #[serde(default)]
    pub tcp_port: Vec<PortPoolConfig>,
}

impl ResourcePoolsConfig {
    pub fn new() -> Self {
        Self::default()
    }
    /// Load config from a YAML file.
    pub fn load_yaml_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).with_context(|| format!("read config file: {}", path.display()))?;
        let cfg: Self = serde_yaml::from_slice(&bytes)
            .with_context(|| format!("parse yaml config: {}", path.display()))?;
        Ok(cfg)
    }

    /// Build runtime pools from this config, performing validation and expansion.
    pub fn build(&self) -> anyhow::Result<ResourcePools> {
        ResourcePools::from_config(self)
    }
}
