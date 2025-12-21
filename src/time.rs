use parking_lot::Mutex;
use std::any::Any;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerToken(u64);

impl TimerToken {
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn raw(&self) -> u64 {
        self.0
    }
}

/// Time manager abstraction (will have hybrid coarse+fine impl in P1)
pub trait TimeManager: Send + Sync + 'static {
    fn now(&self) -> Instant;
    fn schedule_at(&self, at: Instant, token: TimerToken);
    fn cancel(&self, token: TimerToken) -> bool;

    fn as_any(&self) -> &dyn Any;
}

/// Simple polling placeholder for P0/P1 bootstrapping (no real scheduling yet)
pub struct PollTimeManager {
    counter: AtomicU64,
    timer: Arc<InMemoryTimer>,
}

impl PollTimeManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            counter: AtomicU64::new(0),
            timer: Arc::new(InMemoryTimer::new()),
        })
    }
    pub fn with_timer(timer: Arc<InMemoryTimer>) -> Arc<Self> {
        Arc::new(Self {
            counter: AtomicU64::new(0),
            timer,
        })
    }
    pub fn next_token(&self) -> TimerToken {
        TimerToken(self.counter.fetch_add(1, Ordering::Relaxed) + 1)
    }
    pub fn timer(&self) -> Arc<InMemoryTimer> {
        self.timer.clone()
    }
}

impl TimeManager for PollTimeManager {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn schedule_at(&self, at: Instant, token: TimerToken) {
        self.timer.schedule(at, token);
    }
    fn cancel(&self, token: TimerToken) -> bool {
        self.timer.cancel(token)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Hybrid time manager (coarse sleep + fine spin) skeleton for reduced drift.
pub struct HybridTimeManager {
    counter: AtomicU64,
    timer: Arc<InMemoryTimer>,
    // Adaptive tuning targets (protected by mutex for infrequent writes)
    spin_threshold: Mutex<Duration>,
    max_sleep_slice: Mutex<Duration>,
    // Moving average (EMA) of drift in nanoseconds
    ema_drift_ns: AtomicU64,
    sample_count: AtomicU64,
    adapt_enabled: bool,
}

impl HybridTimeManager {
    pub const DEFAULT_SPIN_THRESHOLD: Duration = Duration::from_micros(200);
    pub const DEFAULT_MAX_SLEEP_SLICE: Duration = Duration::from_millis(5);
    const SPIN_MIN: Duration = Duration::from_micros(50);
    const SPIN_MAX: Duration = Duration::from_micros(500);
    const SLICE_MIN: Duration = Duration::from_millis(1);
    const SLICE_MAX: Duration = Duration::from_millis(10);
    const ADAPT_EVERY: u64 = 128; // samples interval
    const EMA_ALPHA_NUM: u64 = 2; // alpha ≈ 2/10 = 0.2
    const EMA_ALPHA_DEN: u64 = 10;

    pub fn new(spin_threshold: Duration, max_sleep_slice: Duration) -> Arc<Self> {
        // Allow environment override HYBRID_STATIC=1 to disable adaptation.
        let static_flag = std::env::var("HYBRID_STATIC").ok().as_deref() == Some("1");
        Arc::new(Self {
            counter: AtomicU64::new(0),
            timer: Arc::new(InMemoryTimer::new()),
            spin_threshold: Mutex::new(spin_threshold),
            max_sleep_slice: Mutex::new(max_sleep_slice),
            ema_drift_ns: AtomicU64::new(0),
            sample_count: AtomicU64::new(0),
            adapt_enabled: !static_flag,
        })
    }
    pub fn new_static(spin_threshold: Duration, max_sleep_slice: Duration) -> Arc<Self> {
        Arc::new(Self {
            counter: AtomicU64::new(0),
            timer: Arc::new(InMemoryTimer::new()),
            spin_threshold: Mutex::new(spin_threshold),
            max_sleep_slice: Mutex::new(max_sleep_slice),
            ema_drift_ns: AtomicU64::new(0),
            sample_count: AtomicU64::new(0),
            adapt_enabled: false,
        })
    }
    pub fn new_default() -> Arc<Self> {
        Self::new(Self::DEFAULT_SPIN_THRESHOLD, Self::DEFAULT_MAX_SLEEP_SLICE)
    }
    pub fn next_token(&self) -> TimerToken {
        TimerToken(self.counter.fetch_add(1, Ordering::Relaxed) + 1)
    }
    pub fn timer(&self) -> Arc<InMemoryTimer> {
        self.timer.clone()
    }

    fn adapt_parameters(&self, ema_ns: u64) {
        let ema = Duration::from_nanos(ema_ns);
        let mut spin = self.spin_threshold.lock();
        let mut slice = self.max_sleep_slice.lock();
        // Heuristics: keep EMA within ~0.5..1.5 of spin threshold
        if ema > *spin * 2 && *spin < Self::SPIN_MAX {
            // drift too large, increase spin window
            let new_spin = (*spin * 5 / 4).min(Self::SPIN_MAX); // +25%
            *spin = new_spin;
        } else if ema * 2 < *spin && *spin > Self::SPIN_MIN {
            // drift small, reduce spin
            let new_spin = (*spin * 4 / 5).max(Self::SPIN_MIN); // -20%
            *spin = new_spin;
        }
        // Adjust coarse slice: if drift >> spin, shorten slice to re-check sooner
        if ema > *spin * 3 && *slice > Self::SLICE_MIN {
            *slice = (*slice * 4 / 5).max(Self::SLICE_MIN); // -20%
        } else if ema < *spin && *slice < Self::SLICE_MAX {
            *slice = (*slice * 11 / 10).min(Self::SLICE_MAX); // +10%
        }
    }

    pub fn spin_threshold_val(&self) -> Duration {
        *self.spin_threshold.lock()
    }
    pub fn max_sleep_slice_val(&self) -> Duration {
        *self.max_sleep_slice.lock()
    }
}

impl Default for HybridTimeManager {
    fn default() -> Self {
        // Provide a Default that matches new(DEFAULTS), for use in tests/benches
        Self {
            counter: AtomicU64::new(0),
            timer: Arc::new(InMemoryTimer::new()),
            spin_threshold: Mutex::new(Self::DEFAULT_SPIN_THRESHOLD),
            max_sleep_slice: Mutex::new(Self::DEFAULT_MAX_SLEEP_SLICE),
            ema_drift_ns: AtomicU64::new(0),
            sample_count: AtomicU64::new(0),
            adapt_enabled: true,
        }
    }
}

impl TimeManager for HybridTimeManager {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn schedule_at(&self, at: Instant, token: TimerToken) {
        self.timer.schedule(at, token);
    }
    fn cancel(&self, token: TimerToken) -> bool {
        self.timer.cancel(token)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Clone)]
pub struct TimerRequest {
    pub at: Instant,
    pub token: TimerToken,
}

impl PartialEq for TimerRequest {
    fn eq(&self, other: &Self) -> bool {
        self.at == other.at && self.token == other.token
    }
}
impl Eq for TimerRequest {}
impl PartialOrd for TimerRequest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TimerRequest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.at.cmp(&self.at)
    }
}

pub struct InMemoryTimer {
    heap: Mutex<BinaryHeap<TimerRequest>>, // min-heap via Ord impl (reverse)
    cancelled: Mutex<HashMap<u64, bool>>,  // token -> cancelled
}

impl Default for InMemoryTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryTimer {
    pub fn new() -> Self {
        Self {
            heap: Mutex::new(BinaryHeap::new()),
            cancelled: Mutex::new(HashMap::new()),
        }
    }

    pub fn schedule(&self, at: Instant, token: TimerToken) {
        self.heap.lock().push(TimerRequest { at, token });
    }

    pub fn cancel(&self, token: TimerToken) -> bool {
        self.cancelled.lock().insert(token.raw(), true).is_some()
    }

    pub fn pop_due(&self, now: Instant) -> Vec<(TimerToken, Instant)> {
        let mut heap = self.heap.lock();
        let mut out = Vec::new();
        while let Some(top) = heap.peek() {
            if top.at <= now {
                let t = heap.pop().unwrap();
                if !self.cancelled.lock().contains_key(&t.token.raw()) {
                    out.push((t.token, t.at));
                } else {
                    // cleanup cancelled entry (since we encountered it)
                    self.cancelled.lock().remove(&t.token.raw());
                }
            } else {
                break;
            }
        }
        // Opportunistic cleanup if map is large
        if self.cancelled.lock().len() > 1024 {
            // rebuild smaller map by retaining only keys that still exist in heap (rare path)
            let mut live = HashMap::new();
            for req in heap.iter() {
                if let Some(v) = self.cancelled.lock().get(&req.token.raw()) {
                    live.insert(req.token.raw(), *v);
                }
            }
            *self.cancelled.lock() = live;
        }
        out
    }

    /// Peek the next earliest deadline without popping.
    pub fn peek_next_at(&self) -> Option<Instant> {
        self.heap.lock().peek().map(|r| r.at)
    }
}

pub struct TimerDriver<T: TimeManager> {
    pub tm: Arc<T>,
    inner: Arc<InMemoryTimer>,
    running: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>, // new
}

impl<T: TimeManager> TimerDriver<T> {
    pub fn new(tm: Arc<T>, inner: Arc<InMemoryTimer>) -> Self {
        Self {
            tm,
            inner,
            running: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
        }
    }

    pub fn spawn<F>(&self, interval_check: Duration, mut on_fire: F)
    where
        F: FnMut(TimerToken, Instant) + Send + 'static,
    {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let tm = self.tm.clone();
        let inner = self.inner.clone();
        let running = self.running.clone();
        let handle = thread::spawn(move || {
            while running.load(Ordering::Relaxed) {
                let now = tm.now();
                let due = inner.pop_due(now);
                for (token, at) in due {
                    on_fire(token, at);
                }
                thread::sleep(interval_check);
            }
        });
        *self.handle.lock() = Some(handle);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.lock().take() {
            let _ = h.join();
        }
    }
}
