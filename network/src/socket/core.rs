use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A protocol-agnostic connection key.
///
/// This trait exists to let the socket layer grow beyond UDP (TCP/RAW/ETH) without
/// baking a single protocol's 4-tuple into the generic API surface.
#[allow(dead_code)]
pub trait ConnKey: Copy + Eq + std::hash::Hash {
    /// Human-readable protocol tag, mainly for debugging/metrics.
    fn proto_name(&self) -> &'static str;
}

/// A protocol-agnostic connection entry.
///
/// The MVP common denominator across protocols is:
/// - a key
/// - liveness timestamps (for TTL/eviction)
///
/// Protocol-specific behavior should live in per-protocol impl blocks.
#[allow(dead_code)]
pub trait ConnEntry {
    type Key: ConnKey;

    fn key(&self) -> Self::Key;
    fn created_at(&self) -> Instant;
    fn last_seen(&self) -> Instant;
    fn set_last_seen(&mut self, now: Instant);
}

/// Generic stats for connection tables.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConnTableStats {
    pub lookups: u64,
    pub hits: u64,
    pub inserts: u64,
    pub evictions: u64,
}

/// Configuration for [`ConnTableCore`].
#[derive(Debug, Clone, Copy)]
pub struct ConnTableConfig {
    /// Maximum number of tracked sockets.
    pub max_entries: usize,
    /// Optional TTL; entries older than this are eligible for eviction on insert.
    pub ttl: Option<Duration>,
}

impl Default for ConnTableConfig {
    fn default() -> Self {
        Self {
            max_entries: 1024,
            ttl: Some(Duration::from_secs(60)),
        }
    }
}

/// A protocol-agnostic connection table.
///
/// This type only implements generic behaviors shared across protocols:
/// - keyed lookup
/// - liveness refresh timestamps
/// - TTL/cap eviction
/// - generic stats
///
/// Per-protocol workflows should live in per-protocol impl blocks.
#[derive(Debug)]
pub struct ConnTableCore<C: ConnEntry> {
    pub(crate) cfg: ConnTableConfig,
    pub(crate) map: HashMap<C::Key, C>,
    pub(crate) stats: ConnTableStats,
}

impl<C: ConnEntry> Default for ConnTableCore<C> {
    fn default() -> Self {
        Self::new(ConnTableConfig::default())
    }
}

impl<C: ConnEntry> ConnTableCore<C> {
    pub fn new(cfg: ConnTableConfig) -> Self {
        Self {
            cfg,
            map: HashMap::new(),
            stats: ConnTableStats::default(),
        }
    }

    pub fn stats(&self) -> ConnTableStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn get(&mut self, key: &C::Key) -> Option<&C> {
        self.stats.lookups += 1;
        if self.map.contains_key(key) {
            self.stats.hits += 1;
        }
        self.map.get(key)
    }

    pub fn get_mut(&mut self, key: &C::Key) -> Option<&mut C> {
        self.stats.lookups += 1;
        if self.map.contains_key(key) {
            self.stats.hits += 1;
        }
        self.map.get_mut(key)
    }

    pub fn remove(&mut self, key: &C::Key) -> Option<C> {
        self.map.remove(key)
    }

    pub fn insert(&mut self, key: C::Key, value: C) -> Option<C> {
        self.evict_if_needed();
        let prev = self.map.insert(key, value);
        self.stats.inserts += 1;
        prev
    }

    pub fn contains_key(&self, key: &C::Key) -> bool {
        self.map.contains_key(key)
    }

    pub(crate) fn evict_if_needed(&mut self) {
        if let Some(ttl) = self.cfg.ttl {
            let now = Instant::now();
            let mut expired: Vec<C::Key> = Vec::new();
            for (k, v) in self.map.iter() {
                if now.duration_since(v.last_seen()) > ttl {
                    expired.push(*k);
                }
            }
            for k in expired {
                if self.map.remove(&k).is_some() {
                    self.stats.evictions += 1;
                }
            }
        }

        if self.map.len() >= self.cfg.max_entries {
            if let Some((oldest_key, _)) = self
                .map
                .iter()
                .min_by_key(|(_k, v)| v.last_seen())
                .map(|(k, v)| (*k, v.last_seen()))
            {
                if self.map.remove(&oldest_key).is_some() {
                    self.stats.evictions += 1;
                }
            }
        }
    }
}
