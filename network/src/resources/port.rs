use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::OwnerId;
use super::PortProtocol;

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
    pub(crate) owner_to_pinned: BTreeMap<OwnerId, BTreeSet<u16>>,
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

    pub fn acquire_for(&mut self, owner: &OwnerId) -> Option<u16> {
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
                self.owner_rr.insert(owner.clone(), (idx + 1) % len);
                return Some(p);
            }
        }

        // All pinned ports are already leased; fall back to regular acquire.
        self.acquire()
    }

    pub fn pin(&mut self, owner: OwnerId, port: u16) -> anyhow::Result<()> {
        let owner = owner;

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
    pub fn unpin(&mut self, owner: &OwnerId, port: u16) -> bool {
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

    pub fn unpin_owner(&mut self, owner: &OwnerId) -> bool {
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

    pub(crate) fn from_config(cfgs: &[PortPoolConfig]) -> anyhow::Result<Self> {
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
