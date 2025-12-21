use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A protocol-agnostic connection key.
///
/// `socket::core` 是纯容器层：这里只关心“可哈希、可比较、可复制”的 key。
///
/// 注意：这里故意不要求 `proto_name()` 之类的语义信息，避免 core 被协议语义污染。
#[allow(dead_code)]
pub trait ConnKey: Copy + Eq + std::hash::Hash {}

// Blanket impl: 满足 Copy+Eq+Hash 的类型自动成为 ConnKey。
impl<T> ConnKey for T where T: Copy + Eq + std::hash::Hash {}

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

/// Optional capability for a connection key to expose a stable process-local id.
///
/// This enables `ConnTableCore` to keep a secondary index that can resolve
/// `sock_id -> key -> entry` without changing the primary key semantics.
pub trait HasSockId {
    fn sock_id(&self) -> u64;
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
    pub(crate) by_sock_id: HashMap<u64, C::Key>,
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
            by_sock_id: HashMap::new(),
            stats: ConnTableStats::default(),
        }
    }

    /// 只读访问配置。
    pub fn config(&self) -> ConnTableConfig {
        self.cfg
    }

    pub fn stats(&self) -> ConnTableStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 查询（计入 stats）。
    pub fn get(&mut self, key: &C::Key) -> Option<&C> {
        self.stats.lookups += 1;
        if self.map.contains_key(key) {
            self.stats.hits += 1;
        }
        self.map.get(key)
    }

    /// 只读 peek（不计入 stats）——用于调试/打印等不想污染指标的场景。
    pub fn peek(&self, key: &C::Key) -> Option<&C> {
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
        let removed = self.map.remove(key);
        if removed.is_some() {
            // Also remove any sock_id mapping pointing to this key.
            self.by_sock_id.retain(|_, v| v != key);
        }
        removed
    }

    pub fn insert(&mut self, key: C::Key, value: C) -> Option<C> {
        self.evict_if_needed();
        let prev = self.map.insert(key, value);
        self.stats.inserts += 1;
        prev
    }

    /// 获取现有条目（若存在）或使用 `create` 创建并插入。
    ///
    /// 语义：
    /// - 命中：返回可变引用，并由调用者决定是否 refresh last_seen
    /// - 未命中：执行 eviction（TTL+cap），插入新值后返回可变引用
    pub fn get_or_insert_with(&mut self, key: C::Key, create: impl FnOnce() -> C) -> &mut C {
        self.stats.lookups += 1;
        if self.map.contains_key(&key) {
            self.stats.hits += 1;
            return self.map.get_mut(&key).expect("exists");
        }

        self.evict_if_needed();
        self.map.insert(key, create());
        self.stats.inserts += 1;
        self.map.get_mut(&key).expect("inserted")
    }

    /// 与 [`ConnTableCore::get_or_insert_with`] 类似，但会额外返回是否发生了插入。
    ///
    /// 语义：
    /// - 命中：`inserted=false`
    /// - 未命中：执行 eviction（TTL+cap），插入新值，`inserted=true`
    pub fn upsert_with(&mut self, key: C::Key, create: impl FnOnce() -> C) -> (&mut C, bool) {
        self.stats.lookups += 1;
        if self.map.contains_key(&key) {
            self.stats.hits += 1;
            return (self.map.get_mut(&key).expect("exists"), false);
        }

        self.evict_if_needed();
        self.map.insert(key, create());
        self.stats.inserts += 1;
        (self.map.get_mut(&key).expect("inserted"), true)
    }

    /// Lookup by sock id (secondary index) without affecting the primary key semantics.
    pub fn get_by_sock_id(&mut self, sock_id: u64) -> Option<&C>
    where
        C::Key: HasSockId,
    {
        let key = self.by_sock_id.get(&sock_id).copied()?;
        self.get(&key)
    }

    /// Debug/peek variant of [`ConnTableCore::get_by_sock_id`] (no stats).
    pub fn peek_by_sock_id(&self, sock_id: u64) -> Option<&C>
    where
        C::Key: HasSockId,
    {
        let key = self.by_sock_id.get(&sock_id).copied()?;
        self.peek(&key)
    }

    /// 获取现有条目或插入新值，但返回不可变引用（避免上层 `peek()+expect()` 的样板）。
    ///
    /// 注意：如果你需要在命中/插入后修改条目（例如刷新 last_seen），请优先使用
    /// [`ConnTableCore::get_or_insert_with`] 或 [`ConnTableCore::upsert_with`]。
    pub fn get_or_insert_with_ref(&mut self, key: C::Key, create: impl FnOnce() -> C) -> &C {
        let _ = self.get_or_insert_with(key, create);
        self.peek(&key).expect("exists")
    }

    pub fn contains_key(&self, key: &C::Key) -> bool {
        self.map.contains_key(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&C::Key, &C)> {
        self.map.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&C::Key, &mut C)> {
        self.map.iter_mut()
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
                self.by_sock_id.retain(|_, v| v != &k);
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
                self.by_sock_id.retain(|_, v| v != &oldest_key);
            }
        }
    }
}

impl<C> ConnTableCore<C>
where
    C: ConnEntry,
    C::Key: HasSockId,
{
    #[inline]
    fn index_sock_id_for_key(&mut self, key: &C::Key) {
        self.by_sock_id.insert(key.sock_id(), *key);
    }

    /// upsert variant that also maintains the sock_id index.
    pub fn upsert_with_sock_id(
        &mut self,
        key: C::Key,
        create: impl FnOnce() -> C,
    ) -> (&mut C, bool) {
        // Fast path: exists.
        self.stats.lookups += 1;
        if self.map.contains_key(&key) {
            self.stats.hits += 1;
            self.index_sock_id_for_key(&key);
            return (self.map.get_mut(&key).expect("exists"), false);
        }

        self.evict_if_needed();
        self.map.insert(key, create());
        self.index_sock_id_for_key(&key);
        self.stats.inserts += 1;
        (self.map.get_mut(&key).expect("inserted"), true)
    }
}
