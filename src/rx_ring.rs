use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use tokio::time;

/// Host-side provider for `ntx:host/rx-ring@0.1.0`.
///
/// This is intentionally self-contained and `std`-only (no tokio), because Wasmtime
/// component imports are currently wired via a sync linker in this repo.
///
/// Contract highlights (see `component/doc/HOST.md`):
/// - Bounded queue
/// - Handle = (slot_id, generation) packed into `u64`
/// - `wait_rx` must:
///   - return `None` on timeout
///   - be woken on enqueue
///   - be woken on shutdown
/// - `read_*` must bounds-check and return stable error strings:
///   - "invalid handle"
///   - "out of bounds"
/// - Lease timeout ensures buffers are eventually reclaimed.
#[derive(Debug, Clone)]
pub struct RxRing {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    state: Mutex<State>,
    notify: Notify,

    cfg: RxRingConfig,
    metrics: RxRingMetrics,
}

#[derive(Debug, Clone)]
pub struct RxRingConfig {
    pub max_queue_depth: usize,
    pub lease_timeout: Duration,
}

impl Default for RxRingConfig {
    fn default() -> Self {
        Self {
            max_queue_depth: 1024,
            lease_timeout: Duration::from_millis(5000),
        }
    }
}

#[derive(Debug, Default)]
pub struct RxRingMetrics {
    pub enqueue_drop_total: std::sync::atomic::AtomicU64,
    pub lease_expired_total: std::sync::atomic::AtomicU64,

    pub wait_wake_total: std::sync::atomic::AtomicU64,
    pub wait_timeout_total: std::sync::atomic::AtomicU64,
    pub wait_shutdown_wake_total: std::sync::atomic::AtomicU64,
}

#[derive(Debug)]
struct State {
    shutdown: bool,
    seq: u64,

    /// Queue of ready slot ids (in FIFO order).
    ready: VecDeque<u32>,

    /// Slot store.
    slots: Vec<Slot>,

    /// (slot_id,generation) -> inflight entry.
    inflight: HashMap<u64, Inflight>,

    /// Total bytes in queue (desc+payload) to aid observability.
    bytes_in_queue: u64,
}

#[derive(Debug)]
struct Slot {
    generation: u32,
    desc: Option<Vec<u8>>,
    payload: Option<Vec<u8>>,
}

#[derive(Debug)]
struct Inflight {
    slot_id: u32,
    generation: u32,
    desc_len: u32,
    payload_len: u32,
    deadline: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RxBatch {
    pub handle: u64,
    pub desc_len: u32,
    pub payload_len: u32,
    pub seq: u64,
}

impl RxRing {
    pub fn new(cfg: RxRingConfig) -> Self {
        let inner = Inner {
            state: Mutex::new(State {
                shutdown: false,
                seq: 0,
                ready: VecDeque::new(),
                slots: Vec::new(),
                inflight: HashMap::new(),
                bytes_in_queue: 0,
            }),
            notify: Notify::new(),
            cfg,
            metrics: RxRingMetrics::default(),
        };

        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn metrics(&self) -> &RxRingMetrics {
        &self.inner.metrics
    }

    pub fn shutdown(&self) {
        let mut st = self.inner.state.lock().expect("rx-ring mutex poisoned");
        if !st.shutdown {
            st.shutdown = true;
            self.inner
                .metrics
                .wait_shutdown_wake_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // Wake any `wait_rx`.
        self.inner.notify.notify_waiters();
    }

    /// Enqueue a batch.
    ///
    /// Policy: if queue is full, drop newest (the incoming batch).
    pub fn enqueue_batch(&self, desc: Vec<u8>, payload: Vec<u8>) {
        let mut st = self.inner.state.lock().expect("rx-ring mutex poisoned");
        if st.shutdown {
            // If shutting down, just drop silently.
            return;
        }

        if st.ready.len() >= self.inner.cfg.max_queue_depth {
            self.inner
                .metrics
                .enqueue_drop_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }

        self.expire_leases_locked(&mut st);

        let slot_id = self.alloc_slot_locked(&mut st);
        let generation = st.slots[slot_id as usize].generation;

        let desc_len = desc.len() as u32;
        let payload_len = payload.len() as u32;

        st.bytes_in_queue = st
            .bytes_in_queue
            .saturating_add(desc_len as u64 + payload_len as u64);

        st.slots[slot_id as usize].desc = Some(desc);
        st.slots[slot_id as usize].payload = Some(payload);
        st.ready.push_back(slot_id);

        // Wake up any waiters.
        self.inner
            .metrics
            .wait_wake_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.notify.notify_one();

        // Note: handle is not created until a batch is dequeued into inflight.
        let _ = generation;
    }

    pub fn queue_depth(&self) -> usize {
        let st = self.inner.state.lock().expect("rx-ring mutex poisoned");
        st.ready.len()
    }

    pub fn inflight_handles(&self) -> usize {
        let st = self.inner.state.lock().expect("rx-ring mutex poisoned");
        st.inflight.len()
    }

    pub fn bytes_in_queue(&self) -> u64 {
        let st = self.inner.state.lock().expect("rx-ring mutex poisoned");
        st.bytes_in_queue
    }

    pub fn poll_rx(&self, max_desc: u32, max_payload: u32) -> Option<RxBatch> {
        let mut st = self.inner.state.lock().expect("rx-ring mutex poisoned");
        if st.shutdown {
            return None;
        }
        self.expire_leases_locked(&mut st);
        self.dequeue_one_locked(&mut st, max_desc, max_payload)
    }

    pub fn wait_rx(&self, max_desc: u32, max_payload: u32, timeout_ms: u32) -> Option<RxBatch> {
        // Compatibility shim: sync callers still exist (WIT host import traits currently
        // sync in this repo). Prefer `wait_rx_async` in Tokio contexts.
        tokio::runtime::Handle::try_current()
            .ok()
            .map(|h| h.block_on(self.wait_rx_async(max_desc, max_payload, timeout_ms)))
            .unwrap_or_else(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build tokio runtime")
                    .block_on(self.wait_rx_async(max_desc, max_payload, timeout_ms))
            })
    }

    /// Async wait variant used by Tokio-native host paths.
    ///
    /// Semantics:
    /// - returns `None` on timeout
    /// - returns `None` on shutdown
    /// - wakes when `enqueue_batch` is called
    pub async fn wait_rx_async(
        &self,
        max_desc: u32,
        max_payload: u32,
        timeout_ms: u32,
    ) -> Option<RxBatch> {
        let timeout = Duration::from_millis(timeout_ms as u64);

        // Fast path: try to dequeue under lock.
        {
            let mut st = self.inner.state.lock().expect("rx-ring mutex poisoned");
            if st.shutdown {
                return None;
            }
            self.expire_leases_locked(&mut st);
            if let Some(b) = self.dequeue_one_locked(&mut st, max_desc, max_payload) {
                return Some(b);
            }
        }

        // Slow path: wait for either enqueue/shutdown or timeout.
        let notified = self.inner.notify.notified();
        if time::timeout(timeout, notified).await.is_err() {
            self.inner
                .metrics
                .wait_timeout_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return None;
        }

        // Re-check under lock after wake.
        let mut st = self.inner.state.lock().expect("rx-ring mutex poisoned");
        if st.shutdown {
            return None;
        }
        self.expire_leases_locked(&mut st);
        self.dequeue_one_locked(&mut st, max_desc, max_payload)
    }

    pub fn read_desc(&self, handle: u64, off: u32, len: u32) -> Result<Vec<u8>, String> {
        self.read_buf(handle, off, len, true)
    }

    pub fn read_payload(&self, handle: u64, off: u32, len: u32) -> Result<Vec<u8>, String> {
        self.read_buf(handle, off, len, false)
    }

    pub fn release(&self, handle: u64) -> Result<(), String> {
        let mut st = self.inner.state.lock().expect("rx-ring mutex poisoned");
        self.expire_leases_locked(&mut st);

        let Some(inf) = st.inflight.remove(&handle) else {
            return Err("invalid handle".to_string());
        };

        // Free buffers in slot.
        // Generation stays; increments on reuse.
        let (desc_len, payload_len) = {
            let slot = &st.slots[inf.slot_id as usize];
            (
                slot.desc.as_ref().map(|v| v.len() as u64).unwrap_or(0),
                slot.payload.as_ref().map(|v| v.len() as u64).unwrap_or(0),
            )
        };

        st.bytes_in_queue = st
            .bytes_in_queue
            .saturating_sub(desc_len.saturating_add(payload_len));

        let slot = &mut st.slots[inf.slot_id as usize];
        slot.desc = None;
        slot.payload = None;

        Ok(())
    }

    fn read_buf(&self, handle: u64, off: u32, len: u32, is_desc: bool) -> Result<Vec<u8>, String> {
        let mut st = self.inner.state.lock().expect("rx-ring mutex poisoned");
        self.expire_leases_locked(&mut st);

        let Some(inf) = st.inflight.get(&handle) else {
            return Err("invalid handle".to_string());
        };

        // Verify generation matches slot generation.
        let slot = &st.slots[inf.slot_id as usize];
        if slot.generation != inf.generation {
            return Err("invalid handle".to_string());
        }

        let total = if is_desc {
            inf.desc_len
        } else {
            inf.payload_len
        };
        let end = off
            .checked_add(len)
            .ok_or_else(|| "out of bounds".to_string())?;
        if end > total {
            return Err("out of bounds".to_string());
        }

        let buf = if is_desc {
            slot.desc
                .as_ref()
                .ok_or_else(|| "invalid handle".to_string())?
        } else {
            slot.payload
                .as_ref()
                .ok_or_else(|| "invalid handle".to_string())?
        };

        let start = off as usize;
        let end = end as usize;
        Ok(buf[start..end].to_vec())
    }

    fn dequeue_one_locked(
        &self,
        st: &mut State,
        max_desc: u32,
        max_payload: u32,
    ) -> Option<RxBatch> {
        // Find first batch that satisfies max limits (no truncation).
        let mut i = 0usize;
        while i < st.ready.len() {
            let slot_id = st.ready[i];
            let slot = &st.slots[slot_id as usize];
            let desc_len = slot.desc.as_ref().map(|v| v.len() as u32).unwrap_or(0);
            let payload_len = slot.payload.as_ref().map(|v| v.len() as u32).unwrap_or(0);

            if desc_len <= max_desc && payload_len <= max_payload {
                // Remove from ready queue.
                st.ready.remove(i);

                st.seq = st.seq.wrapping_add(1);

                let generation = slot.generation;
                let handle = pack_handle(slot_id, generation);

                let deadline = Instant::now() + self.inner.cfg.lease_timeout;
                st.inflight.insert(
                    handle,
                    Inflight {
                        slot_id,
                        generation,
                        desc_len,
                        payload_len,
                        deadline,
                    },
                );

                return Some(RxBatch {
                    handle,
                    desc_len,
                    payload_len,
                    seq: st.seq,
                });
            }

            // If this batch doesn't fit, keep scanning.
            i += 1;
        }

        None
    }

    fn alloc_slot_locked(&self, st: &mut State) -> u32 {
        // Try to reuse a free slot.
        for (idx, slot) in st.slots.iter_mut().enumerate() {
            if slot.desc.is_none() && slot.payload.is_none() {
                // bump generation to invalidate stale handles
                slot.generation = slot.generation.wrapping_add(1).max(1);
                return idx as u32;
            }
        }
        // else grow.
        let idx = st.slots.len();
        st.slots.push(Slot {
            generation: 1,
            desc: None,
            payload: None,
        });
        idx as u32
    }

    fn expire_leases_locked(&self, st: &mut State) {
        if st.inflight.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut expired: Vec<u64> = Vec::new();
        for (h, inf) in st.inflight.iter() {
            if inf.deadline <= now {
                expired.push(*h);
            }
        }
        if expired.is_empty() {
            return;
        }

        for h in expired {
            if let Some(inf) = st.inflight.remove(&h) {
                let slot = &mut st.slots[inf.slot_id as usize];
                // free buffers
                let desc_len = slot.desc.as_ref().map(|v| v.len() as u64).unwrap_or(0);
                let payload_len = slot.payload.as_ref().map(|v| v.len() as u64).unwrap_or(0);
                st.bytes_in_queue = st
                    .bytes_in_queue
                    .saturating_sub(desc_len.saturating_add(payload_len));
                slot.desc = None;
                slot.payload = None;

                self.inner
                    .metrics
                    .lease_expired_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

#[inline]
fn pack_handle(slot_id: u32, generation: u32) -> u64 {
    ((generation as u64) << 32) | (slot_id as u64)
}

#[allow(dead_code)]
#[inline]
fn unpack_handle(handle: u64) -> (u32, u32) {
    let slot_id = (handle & 0xFFFF_FFFF) as u32;
    let generation = (handle >> 32) as u32;
    (slot_id, generation)
}
