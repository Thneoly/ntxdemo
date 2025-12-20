use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::MacAddr;

use super::OwnerId;
use super::parse::{mac_to_u64, parse_mac, u64_to_mac};

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
    pub(crate) owner_to_pinned: BTreeMap<OwnerId, MacAddr>,
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

    pub fn pin(&mut self, owner: impl Into<OwnerId>, mac: MacAddr) -> anyhow::Result<()> {
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

    pub(crate) fn from_config(cfgs: &[MacPoolConfig]) -> anyhow::Result<Self> {
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
