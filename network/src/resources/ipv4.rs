use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::Ipv4Addr;

use super::OwnerId;
use super::parse::{cidr_host_range, parse_cidr_v4, parse_ipv4};

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
    pub(crate) owner_to_pinned: BTreeMap<OwnerId, Ipv4Addr>,
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
    pub fn acquire_for(&mut self, owner: &OwnerId) -> Option<Ipv4Addr> {
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
    pub fn pin(&mut self, owner: OwnerId, ip: Ipv4Addr) -> anyhow::Result<()> {
        let owner = owner;
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

    pub fn unpin_owner(&mut self, owner: &OwnerId) -> bool {
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

    pub(crate) fn from_config(cfgs: &[Ipv4PoolConfig]) -> anyhow::Result<Self> {
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
