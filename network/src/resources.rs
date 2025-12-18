//! Resource pool management (IP/MAC/Port pools) loaded from a config file.
//!
//! This module is intentionally small and standalone so it can be reused by examples
//! and higher-level components.
//!
//! Design goals:
//! - Deterministic allocation order (stable across runs).
//! - Simple ownership model: allocate → release.
//! - Config-driven: parse a config file at startup to define the available resources.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{Ipv4Addr, MacAddr};
use crate::{abr, abr::BindingOwner};

/// Logical owner id for pin/reserve operations (component id, task id, container id, ...).
pub type OwnerId = String;

/// Protocol selector for port pools.
///
/// Note: current YAML schema prefers top-level `udp_port:` and `tcp_port:` lists.
/// This enum exists for future-proofing and potential in-entry overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Udp,
    Tcp,
}

/// Top-level config schema.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ResourcePoolsConfig {
    #[serde(default)]
    pub ipv4: Vec<Ipv4PoolConfig>,
    #[serde(default)]
    pub mac: Vec<MacPoolConfig>,
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
    /// Load config from a YAML file.
    pub fn load_yaml_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes =
            std::fs::read(path).with_context(|| format!("read config file: {}", path.display()))?;
        let cfg: Self = serde_yaml::from_slice(&bytes)
            .with_context(|| format!("parse yaml config: {}", path.display()))?;
        Ok(cfg)
    }

    /// Build runtime pools from this config, performing validation and expansion.
    pub fn build(&self) -> Result<ResourcePools> {
        ResourcePools::from_config(self)
    }
}

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

    pub fn from_config(cfg: &ResourcePoolsConfig) -> Result<Self> {
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

    /// Convenience: publish ABR view by adding bindings for pinned resources of an owner.
    ///
    /// This is deliberately a small helper for examples; higher-level components may want
    /// to reconcile bindings from multiple sources.
    pub fn publish_abr_for_owner(
        &self,
        store: &mut abr::BindingStore,
        owner: &str,
        abr_owner: BindingOwner,
    ) {
        store.clear();

        // IPv4 pinned allocations.
        for pool in self.ipv4.inner.values() {
            if let Some(ip) = pool.owner_to_pinned.get(owner).copied() {
                let ip_be = u32::from_be_bytes(ip.octets());
                store.add(abr::Binding::ipv4_be(ip_be, abr_owner));
            }
        }

        // Port bindings pinned allocations.
        //
        // If the owner has a pinned IPv4, publish (ip,port) bindings.
        // Otherwise, publish wildcard bindings using ip=0.0.0.0.
        //
        // Note: ABR has a WILDCARD flag, but `BindingStore` currently stores only keys.
        // Using ip=0.0.0.0 is enough for accept() paths that treat it as wildcard.
        let pinned_ip = self
            .ipv4
            .inner
            .values()
            .find_map(|p| p.owner_to_pinned.get(owner).copied());

        let ip_be = pinned_ip
            .map(|ip| u32::from_be_bytes(ip.octets()))
            .unwrap_or(0);

        for pool in self.udp_port.inner.values() {
            if let Some(ports) = pool.owner_to_pinned.get(owner) {
                for &port in ports {
                    store.add(abr::Binding::udp_port_be(ip_be, port, abr_owner));
                }
            }
        }

        for pool in self.tcp_port.inner.values() {
            if let Some(ports) = pool.owner_to_pinned.get(owner) {
                for &port in ports {
                    store.add(abr::Binding::tcp_port_be(ip_be, port, abr_owner));
                }
            }
        }

        abr::store_view(store.snapshot());
    }
}

/// A set of pools keyed by name.
///
/// Convention:
/// - if a config entry doesn't specify `name`, it is grouped under "default".
#[derive(Debug, Clone, Default)]
pub struct NamedPools<T> {
    inner: BTreeMap<String, T>,
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
    fn from_ipv4_configs(cfgs: &[Ipv4PoolConfig]) -> Result<Self> {
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
    fn from_mac_configs(cfgs: &[MacPoolConfig]) -> Result<Self> {
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
    fn from_port_configs(cfgs: &[PortPoolConfig]) -> Result<Self> {
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

// ---------------- IP pool ----------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ipv4PoolConfig {
    /// Optional pool name for debugging.
    #[serde(default)]
    pub name: String,

    /// CIDR (e.g. "10.0.0.0/24").
    pub cidr: String,

    /// Optional explicit exclusions.
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Ipv4Pool {
    available: VecDeque<Ipv4Addr>,
    leased: BTreeSet<Ipv4Addr>,
    pinned: BTreeMap<Ipv4Addr, OwnerId>,
    owner_to_pinned: BTreeMap<OwnerId, Ipv4Addr>,
}

impl Ipv4Pool {
    pub fn empty() -> Self {
        Self {
            available: VecDeque::new(),
            leased: BTreeSet::new(),
            pinned: BTreeMap::new(),
            owner_to_pinned: BTreeMap::new(),
        }
    }

    pub fn len_available(&self) -> usize {
        self.available.len()
    }

    pub fn len_leased(&self) -> usize {
        self.leased.len()
    }

    pub fn acquire(&mut self) -> Option<Ipv4Addr> {
        let ip = self.available.pop_front()?;
        self.leased.insert(ip);
        Some(ip)
    }

    /// Acquire from a pinned allocation if owner has one, otherwise allocate normally.
    pub fn acquire_for(&mut self, owner: &str) -> Option<Ipv4Addr> {
        if let Some(ip) = self.owner_to_pinned.get(owner).copied() {
            // If pinned IP is currently free, lease it. If it is already leased, still return it.
            // This keeps semantics simple: pin means “this owner gets this resource”.
            if self.pinned.contains_key(&ip) {
                self.leased.insert(ip);
                return Some(ip);
            }
        }
        self.acquire()
    }

    /// Pin a specific IP for an owner.
    ///
    /// If the IP is free in this pool, it is removed from the available queue.
    pub fn pin(&mut self, owner: impl Into<OwnerId>, ip: Ipv4Addr) -> Result<()> {
        let owner = owner.into();
        if let Some(existing) = self.owner_to_pinned.get(&owner) {
            anyhow::bail!("owner already pinned to {existing:?}");
        }

        // Ensure `ip` exists either in available or leased.
        let exists = self.leased.contains(&ip) || self.available.iter().any(|x| *x == ip);
        anyhow::ensure!(exists, "ip not present in pool");

        // Remove from available if present.
        if let Some(pos) = self.available.iter().position(|x| *x == ip) {
            self.available.remove(pos);
        }

        self.pinned.insert(ip, owner.clone());
        self.owner_to_pinned.insert(owner, ip);
        Ok(())
    }

    pub fn unpin_owner(&mut self, owner: &str) -> bool {
        let Some(ip) = self.owner_to_pinned.remove(owner) else {
            return false;
        };
        self.pinned.remove(&ip);
        // If not leased, return to available.
        if !self.leased.contains(&ip) {
            self.available.push_back(ip);
        }
        true
    }

    pub fn release(&mut self, ip: Ipv4Addr) -> bool {
        if !self.leased.remove(&ip) {
            return false;
        }

        // If pinned, do not return to the general pool.
        if self.pinned.contains_key(&ip) {
            return true;
        }

        self.available.push_back(ip);
        true
    }

    fn from_config(cfgs: &[Ipv4PoolConfig]) -> Result<Self> {
        let mut all = BTreeSet::<Ipv4Addr>::new();

        for cfg in cfgs {
            let (net, prefix) = parse_cidr_v4(&cfg.cidr)
                .with_context(|| format!("invalid ipv4 cidr: {}", cfg.cidr))?;

            // Expand CIDR (excluding network/broadcast by default if prefix <= 30).
            let (start, end) = cidr_host_range(net, prefix);
            for ip_be in start..=end {
                all.insert(Ipv4Addr(ip_be.to_be_bytes()));
            }

            for ex in &cfg.exclude {
                let ip = parse_ipv4(ex).with_context(|| format!("invalid exclude ipv4: {ex}"))?;
                all.remove(&ip);
            }
        }

        let available = VecDeque::from_iter(all.into_iter());
        Ok(Self {
            available,
            leased: BTreeSet::new(),
            pinned: BTreeMap::new(),
            owner_to_pinned: BTreeMap::new(),
        })
    }
}

// ---------------- MAC pool ----------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MacPoolConfig {
    #[serde(default)]
    pub name: String,

    /// Inclusive range start, e.g. "02:00:00:00:00:00".
    pub start: String,

    /// Inclusive range end, e.g. "02:00:00:00:00:ff".
    pub end: String,

    /// Optional explicit exclusions.
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MacPool {
    available: VecDeque<MacAddr>,
    leased: BTreeSet<MacAddr>,
    pinned: BTreeMap<MacAddr, OwnerId>,
    owner_to_pinned: BTreeMap<OwnerId, MacAddr>,
}

impl MacPool {
    pub fn empty() -> Self {
        Self {
            available: VecDeque::new(),
            leased: BTreeSet::new(),
            pinned: BTreeMap::new(),
            owner_to_pinned: BTreeMap::new(),
        }
    }

    pub fn acquire(&mut self) -> Option<MacAddr> {
        let mac = self.available.pop_front()?;
        self.leased.insert(mac);
        Some(mac)
    }

    pub fn acquire_for(&mut self, owner: &str) -> Option<MacAddr> {
        if let Some(mac) = self.owner_to_pinned.get(owner).copied() {
            if self.pinned.contains_key(&mac) {
                self.leased.insert(mac);
                return Some(mac);
            }
        }
        self.acquire()
    }

    pub fn pin(&mut self, owner: impl Into<OwnerId>, mac: MacAddr) -> Result<()> {
        let owner = owner.into();
        if let Some(existing) = self.owner_to_pinned.get(&owner) {
            anyhow::bail!("owner already pinned to {existing:?}");
        }

        let exists = self.leased.contains(&mac) || self.available.iter().any(|x| *x == mac);
        anyhow::ensure!(exists, "mac not present in pool");

        if let Some(pos) = self.available.iter().position(|x| *x == mac) {
            self.available.remove(pos);
        }

        self.pinned.insert(mac, owner.clone());
        self.owner_to_pinned.insert(owner, mac);
        Ok(())
    }

    pub fn unpin_owner(&mut self, owner: &str) -> bool {
        let Some(mac) = self.owner_to_pinned.remove(owner) else {
            return false;
        };
        self.pinned.remove(&mac);
        if !self.leased.contains(&mac) {
            self.available.push_back(mac);
        }
        true
    }

    pub fn release(&mut self, mac: MacAddr) -> bool {
        if !self.leased.remove(&mac) {
            return false;
        }

        if self.pinned.contains_key(&mac) {
            return true;
        }

        self.available.push_back(mac);
        true
    }

    fn from_config(cfgs: &[MacPoolConfig]) -> Result<Self> {
        let mut all = BTreeSet::<MacAddr>::new();

        for cfg in cfgs {
            let start = parse_mac(&cfg.start)
                .with_context(|| format!("invalid mac start: {}", cfg.start))?;
            let end =
                parse_mac(&cfg.end).with_context(|| format!("invalid mac end: {}", cfg.end))?;

            let start_u = mac_to_u64(start);
            let end_u = mac_to_u64(end);
            anyhow::ensure!(start_u <= end_u, "mac range start must be <= end");

            for v in start_u..=end_u {
                all.insert(u64_to_mac(v));
            }

            for ex in &cfg.exclude {
                let mac = parse_mac(ex).with_context(|| format!("invalid exclude mac: {ex}"))?;
                all.remove(&mac);
            }
        }

        let available = VecDeque::from_iter(all.into_iter());
        Ok(Self {
            available,
            leased: BTreeSet::new(),
            pinned: BTreeMap::new(),
            owner_to_pinned: BTreeMap::new(),
        })
    }
}

// ---------------- Port pool ----------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PortPoolConfig {
    #[serde(default)]
    pub name: String,

    /// Optional protocol tag (purely informational for now).
    ///
    /// The current schema selects protocol via top-level keys (`udp_port`/`tcp_port`).
    #[serde(default)]
    pub protocol: Option<PortProtocol>,

    /// Inclusive range start.
    pub start: u16,

    /// Inclusive range end.
    pub end: u16,

    /// Optional explicit exclusions.
    #[serde(default)]
    pub exclude: Vec<u16>,
}

#[derive(Debug, Clone)]
pub struct PortPool {
    available: VecDeque<u16>,
    leased: BTreeSet<u16>,
    pinned: BTreeMap<u16, OwnerId>,
    owner_to_pinned: BTreeMap<OwnerId, BTreeSet<u16>>,
    // Round-robin cursor per owner for pinned ports.
    owner_rr: BTreeMap<OwnerId, usize>,
}

impl PortPool {
    pub fn empty() -> Self {
        Self {
            available: VecDeque::new(),
            leased: BTreeSet::new(),
            pinned: BTreeMap::new(),
            owner_to_pinned: BTreeMap::new(),
            owner_rr: BTreeMap::new(),
        }
    }

    pub fn acquire(&mut self) -> Option<u16> {
        let p = self.available.pop_front()?;
        self.leased.insert(p);
        Some(p)
    }

    pub fn acquire_for(&mut self, owner: &str) -> Option<u16> {
        let Some(ports) = self.owner_to_pinned.get(owner) else {
            return self.acquire();
        };

        if ports.is_empty() {
            return self.acquire();
        }

        // Round-robin over pinned ports, skipping the ones already leased.
        let start = *self.owner_rr.get(owner).unwrap_or(&0);
        let len = ports.len();

        // Make a stable indexable view.
        // (Pinned ports are kept in a BTreeSet so ordering is deterministic.)
        let pinned: Vec<u16> = ports.iter().copied().collect();

        for off in 0..len {
            let idx = (start + off) % len;
            let p = pinned[idx];

            // Sanity: if we lost the pin mapping, ignore it.
            if !self.pinned.contains_key(&p) {
                continue;
            }

            if !self.leased.contains(&p) {
                self.leased.insert(p);
                self.owner_rr.insert(owner.to_string(), (idx + 1) % len);
                return Some(p);
            }
        }

        // All pinned ports are already leased; fall back to regular acquire.
        self.acquire()
    }

    pub fn pin(&mut self, owner: impl Into<OwnerId>, port: u16) -> Result<()> {
        let owner = owner.into();

        if let Some(existing_owner) = self.pinned.get(&port) {
            anyhow::bail!("port already pinned by owner {existing_owner}");
        }

        let exists = self.leased.contains(&port) || self.available.iter().any(|x| *x == port);
        anyhow::ensure!(exists, "port not present in pool");

        if let Some(pos) = self.available.iter().position(|x| *x == port) {
            self.available.remove(pos);
        }

        self.pinned.insert(port, owner.clone());
        self.owner_to_pinned.entry(owner).or_default().insert(port);
        Ok(())
    }

    /// Unpin a single port for an owner.
    pub fn unpin(&mut self, owner: &str, port: u16) -> bool {
        match self.pinned.get(&port) {
            Some(o) if o == owner => {}
            _ => return false,
        }

        self.pinned.remove(&port);
        if let Some(ports) = self.owner_to_pinned.get_mut(owner) {
            ports.remove(&port);
            if ports.is_empty() {
                self.owner_to_pinned.remove(owner);
                self.owner_rr.remove(owner);
            }
        }

        if !self.leased.contains(&port) {
            self.available.push_back(port);
        }

        true
    }

    pub fn unpin_owner(&mut self, owner: &str) -> bool {
        let Some(ports) = self.owner_to_pinned.remove(owner) else {
            return false;
        };

        self.owner_rr.remove(owner);

        for port in ports {
            self.pinned.remove(&port);
            if !self.leased.contains(&port) {
                self.available.push_back(port);
            }
        }
        true
    }

    pub fn release(&mut self, port: u16) -> bool {
        if !self.leased.remove(&port) {
            return false;
        }

        if self.pinned.contains_key(&port) {
            return true;
        }

        self.available.push_back(port);
        true
    }

    fn from_config(cfgs: &[PortPoolConfig]) -> Result<Self> {
        let mut all = BTreeSet::<u16>::new();
        for cfg in cfgs {
            anyhow::ensure!(cfg.start <= cfg.end, "port range start must be <= end");
            for p in cfg.start..=cfg.end {
                all.insert(p);
            }
            for ex in &cfg.exclude {
                all.remove(ex);
            }
        }
        let available = VecDeque::from_iter(all.into_iter());
        Ok(Self {
            available,
            leased: BTreeSet::new(),
            pinned: BTreeMap::new(),
            owner_to_pinned: BTreeMap::new(),
            owner_rr: BTreeMap::new(),
        })
    }
}

// ---------------- parsing helpers ----------------

fn parse_ipv4(s: &str) -> Result<Ipv4Addr> {
    let parts: Vec<&str> = s.split('.').collect();
    anyhow::ensure!(parts.len() == 4, "ipv4 must have 4 octets");
    let mut octets = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        octets[i] = p
            .parse::<u8>()
            .with_context(|| format!("invalid ipv4 octet: {p}"))?;
    }
    Ok(Ipv4Addr(octets))
}

fn parse_mac(s: &str) -> Result<MacAddr> {
    let parts: Vec<&str> = s.split(':').collect();
    anyhow::ensure!(parts.len() == 6, "mac must have 6 bytes");
    let mut octets = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        octets[i] = u8::from_str_radix(p, 16).with_context(|| format!("invalid mac byte: {p}"))?;
    }
    Ok(MacAddr(octets))
}

fn parse_cidr_v4(cidr: &str) -> Result<(u32, u8)> {
    let (ip_s, prefix_s) = cidr
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("cidr must be like a.b.c.d/p"))?;
    let ip = parse_ipv4(ip_s)?;
    let prefix: u8 = prefix_s.parse::<u8>().with_context(|| "invalid prefix")?;
    anyhow::ensure!(prefix <= 32, "prefix must be <= 32");
    Ok((u32::from_be_bytes(ip.octets()), prefix))
}

/// Return the inclusive host range [start, end] in network-order u32.
///
/// Rules:
/// - /32 => single address
/// - /31 => two addresses (RFC 3021) (no network/broadcast distinction)
/// - <= /30 => exclude network and broadcast (typical host pool semantics)
fn cidr_host_range(net_be: u32, prefix: u8) -> (u32, u32) {
    if prefix == 32 {
        return (net_be, net_be);
    }

    let host_bits = 32 - prefix as u32;
    let mask = if prefix == 0 {
        0u32
    } else {
        (!0u32) << host_bits
    };
    let net = net_be & mask;
    let broadcast = net | (!mask);

    if prefix == 31 {
        return (net, broadcast);
    }

    (net.wrapping_add(1), broadcast.wrapping_sub(1))
}

fn mac_to_u64(mac: MacAddr) -> u64 {
    let b = mac.0;
    ((b[0] as u64) << 40)
        | ((b[1] as u64) << 32)
        | ((b[2] as u64) << 24)
        | ((b[3] as u64) << 16)
        | ((b[4] as u64) << 8)
        | (b[5] as u64)
}

fn u64_to_mac(v: u64) -> MacAddr {
    MacAddr([
        ((v >> 40) & 0xff) as u8,
        ((v >> 32) & 0xff) as u8,
        ((v >> 24) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        (v & 0xff) as u8,
    ])
}
