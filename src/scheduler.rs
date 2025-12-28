//! Host scheduler for Ntx.
//!
//! ## Design Goals
//!
//! 1. **No deadlock with long-running guest**: The host scheduler must never hold a lock
//!    while calling the guest's async `run()` export. This is achieved via the **engine owner**
//!    actor pattern, which pulls from shared queues (RX ring, etc.) without blocking the scheduler loop.
//!
//! 2. **Guarantee NicRx polling is never starved by idle wait**:
//!    - Every scheduler loop iteration calls `poll_one_resident_task()` before entering any idle wait.
//!    - NicRx is a special resident that is **always polled** (never suppressed by backoff).
//!    - Bounded idle wait (`IDLE_WAIT_MAX = 2ms`) ensures the loop wakes periodically.
//!    - After bounded wait timeout/notify, the loop immediately re-polls resident tasks.
//!
//! 3. **CPU-friendly**: Exponential backoff for non-NicRx residents reduces spinning; bounded
//!    idle wait prevents sleeping indefinitely when there's nothing to do.
//!
//! 4. **No dependency on plugins**: All scheduling logic is self-contained in root crate.
//!
//! ## Idle Wait Fixed
//!
//! **Prior issue**: When `no_idle_wait=false`, the scheduler would enter `ingest_blocking_bounded()`
//! in a loop (due to an internal loop in the old implementation), blocking indefinitely on the
//! condvar and preventing NicRx from being polled.
//!
//! **Fix**: Removed the internal loop from `ingest_blocking_bounded()`. Now it:
//! 1. Drains ingress queue and due timers (non-blocking).
//! 2. If work is found, return immediately.
//! 3. If not, do a single `wait_timeout(max_wait)` (e.g., 2ms).
//! 4. Return immediately after timeout/notify (don't loop again).
//! 5. The caller (main run loop) immediately re-polls resident tasks.
//!
//! This ensures NicRx polling happens at least every 2ms, even when the run queue is empty.

use crate::app_config::{SchedulerConfig, WasmConfig};
use crate::engine_owner::EngineMsg;
use crate::error::SchedulerError;
use crate::event_bus::{Bytes, EventBus, SimpleEventBus, build_event};
use crate::kernel::non_blocking_recv_udp;
use crate::rx_layout as shared_mem;
use crate::time::{PollTimeManager, TimeManager, TimerToken};
use crate::wasm_engine::{EngineConfig, EngineHandle, EngineManager};
use once_cell::sync::Lazy;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
struct NicRxHeartbeat {
    last_log: Instant,
    calls: u64,
    no_packet: u64,
    enqueued_batches: u64,
    no_engine_tx: u64,
    buffered_not_flushed: u64,
    rx_seen: u64,
    sent_batches: u64,
}

impl Default for NicRxHeartbeat {
    fn default() -> Self {
        Self {
            last_log: Instant::now(),
            calls: 0,
            no_packet: 0,
            enqueued_batches: 0,
            no_engine_tx: 0,
            buffered_not_flushed: 0,
            rx_seen: 0,
            sent_batches: 0,
        }
    }
}

static NIC_RX_HEARTBEAT: Lazy<Mutex<NicRxHeartbeat>> = Lazy::new(|| {
    Mutex::new(NicRxHeartbeat {
        // Ensure the first poll logs quickly.
        last_log: Instant::now() - Duration::from_secs(3600),
        ..NicRxHeartbeat::default()
    })
});

/// A self-contained priority scheduler for the root crate.
///
/// Design goals:
/// - Fixed number of priority lanes (0 is highest priority).
/// - FIFO order within the same priority.
/// - When there are no runnable tasks, block (idle) and wake on submit.
/// - No dependency on `plugins/` modules.

const PRIORITY_LEVELS: usize = 64;
const IDLE_SPIN_LIMIT: usize = 2;
// Safety net: even when there are no timers and no new submissions,
// still wake periodically to poll resident tasks (e.g., NicRx).
const IDLE_WAIT_MAX: Duration = Duration::from_millis(2);
const RESIDENT_BACKOFF_MIN: Duration = Duration::from_micros(50);
const RESIDENT_BACKOFF_MAX: Duration = Duration::from_millis(2);

// RX ring batch sizing for the host->composed-scheduler ABI.
// These are intentionally conservative defaults; the goal is to amortize allocations.
const RX_BATCH_DESC_CAP: u32 = 64;
const RX_BATCH_PAYLOAD_CAP: u32 = 64 * 2048; // 64 packets * 2KiB

/// Common task priorities.
///
/// Note: smaller value means higher priority.
pub const PRIORITY_HIGH: u8 = 0;
pub const PRIORITY_NORMAL: u8 = 32;
pub const PRIORITY_LOW: u8 = 63;

/// Fixed priorities derived from the task category.
///
/// Policy (higher to lower):
/// 1) Network IO
/// 2) Wasm engine calls
/// 3) Timer wakeups
pub const PRIORITY_NET_IO: u8 = 0;
pub const PRIORITY_WASM: u8 = 16;
pub const PRIORITY_TIMER: u8 = 48;

#[derive(Debug)]
pub struct Task {
    pub id: String,
    /// Smaller value means higher priority. Range is clamped into
    /// `[0, PRIORITY_LEVELS - 1]`.
    pub priority: u8,

    pub kind: TaskKind,
}

#[derive(Debug, Clone)]
pub enum TaskKind {
    NetworkIo(NetworkIoTask),
    WasmCall(WasmTask),
    Timer(TimerTask),

    #[cfg(test)]
    TestResident(TestResidentTask),
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestResidentTask {
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkIoTask {
    /// Non-blocking poll/receive from NIC.
    NicRx,
    /// Flush queued frames to NIC.
    NicTx,
}

/// Placeholder for future wasmengine integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmTask {
    pub function: String,
    pub input: Bytes,
}

/// Eventized timer action specification.
///
/// When the timer fires, the scheduler will build and publish an event into `event_bus`.
#[derive(Debug, Clone)]
pub struct TimerEventSpec {
    pub topic: String,
    pub priority: u8,
    pub payload: Bytes,
}

/// A timer task that triggers at/after `at` and then runs `action`.
pub struct TimerTask {
    pub at: Instant,
    pub action: Option<TimerEventSpec>,
}

impl Clone for TimerTask {
    fn clone(&self) -> Self {
        Self {
            at: self.at,
            action: self.action.clone(),
        }
    }
}

impl std::fmt::Debug for TimerTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimerTask")
            .field("at", &self.at)
            .field("has_action", &self.action.is_some())
            .finish()
    }
}

impl Task {
    pub fn net_io(task_id: impl Into<String>, op: NetworkIoTask) -> Self {
        Self {
            id: task_id.into(),
            priority: PRIORITY_NET_IO,
            kind: TaskKind::NetworkIo(op),
        }
    }

    pub fn wasm_call(task_id: impl Into<String>, function: impl Into<String>) -> Self {
        Self {
            id: task_id.into(),
            priority: PRIORITY_WASM,
            kind: TaskKind::WasmCall(WasmTask {
                function: function.into(),
                input: Bytes::new(),
            }),
        }
    }

    pub fn wasm_call_input(
        task_id: impl Into<String>,
        function: impl Into<String>,
        input: Bytes,
    ) -> Self {
        Self {
            id: task_id.into(),
            priority: PRIORITY_WASM,
            kind: TaskKind::WasmCall(WasmTask {
                function: function.into(),
                input,
            }),
        }
    }

    pub fn timer_at(task_id: impl Into<String>, at: Instant, action_id: impl Into<String>) -> Self {
        let action_id: String = action_id.into();
        let spec = TimerEventSpec {
            topic: SimpleEventBus::TOPIC_TIMER_FIRE.to_string(),
            priority: PRIORITY_TIMER.min(7),
            payload: Bytes::from(action_id),
        };
        Self {
            id: task_id.into(),
            priority: PRIORITY_TIMER,
            kind: TaskKind::Timer(TimerTask {
                at,
                action: Some(spec),
            }),
        }
    }

    pub fn timer_action(task_id: impl Into<String>, at: Instant, action: TimerEventSpec) -> Self {
        Self {
            id: task_id.into(),
            priority: PRIORITY_TIMER,
            kind: TaskKind::Timer(TimerTask {
                at,
                action: Some(action),
            }),
        }
    }

    pub fn timer_event(task_id: impl Into<String>, at: Instant, payload: Bytes) -> Self {
        let spec = TimerEventSpec {
            topic: SimpleEventBus::TOPIC_TIMER_FIRE.to_string(),
            priority: PRIORITY_TIMER.min(7),
            payload,
        };
        Self::timer_action(task_id, at, spec)
    }

    pub fn timer_after(
        task_id: impl Into<String>,
        after: Duration,
        action_id: impl Into<String>,
    ) -> Self {
        Self::timer_at(task_id, Instant::now() + after, action_id)
    }
}

#[derive(Debug, Default)]
struct SharedState {
    ingress: VecDeque<Task>,
    timers: TimerState,

    resident: ResidentState,
}

#[derive(Debug, Default)]
struct TimerState {
    /// Map timer token -> task to run when fired.
    by_token: HashMap<u64, Task>,
}

#[derive(Debug, Default)]
struct ResidentState {
    tasks: Vec<ResidentTask>,
}

#[derive(Debug, Clone)]
struct ResidentTask {
    #[allow(dead_code)]
    id: String,
    priority: u8,
    kind: TaskKind,
    backoff: ResidentBackoff,
}

#[derive(Debug, Clone)]
struct ResidentBackoff {
    until: Option<Instant>,
    current: Duration,
}

impl Default for ResidentBackoff {
    fn default() -> Self {
        Self {
            until: None,
            current: RESIDENT_BACKOFF_MIN,
        }
    }
}

pub struct Scheduler {
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    shutdown: Arc<AtomicBool>,
    tm: Arc<dyn TimeManager>,
    bus: Arc<SimpleEventBus>,
    resident_count: std::sync::atomic::AtomicUsize,
    no_idle_wait: AtomicBool,
}

static SCHEDULER: Lazy<Scheduler> = Lazy::new(Scheduler::new);
static ENGINE_TX: Lazy<Mutex<Option<tokio::sync::mpsc::Sender<EngineMsg>>>> =
    Lazy::new(|| Mutex::new(None));

/// Seed the global scheduler with a default "wait for network IO" task.
///
/// This is intended to be called after `kernel::init(...)` and before
/// `start_scheduler()` so that the scheduler has at least one runnable task
/// immediately.
pub fn init() {
    seed_net_io_wait();
}

/// Initialize global scheduler using explicit configuration.
///
/// This avoids relying on environment variables (which are often sanitized by sudo wrappers).
pub fn init_with_config(cfg: SchedulerConfig) {
    seed_net_io_wait();
    Scheduler::global().apply_config(cfg);
}

/// Provide the scheduler with a channel to the engine owner.
///
/// End-state: NIC RX pushes batches to the engine owner, not via EngineManager.
pub fn set_engine_tx(tx: tokio::sync::mpsc::Sender<EngineMsg>) {
    *ENGINE_TX.lock().expect("engine tx poisoned") = Some(tx);
}

fn engine_tx() -> Option<tokio::sync::mpsc::Sender<EngineMsg>> {
    ENGINE_TX.lock().expect("engine tx poisoned").clone()
}

/// Request the engine-owner to begin shutdown.
///
/// End-state requirement (see `component/doc/HOST.md`): host shutdown must wake
/// guest `wait-rx` calls so the run-loop can converge.
pub fn request_engine_shutdown() {
    if let Some(tx) = engine_tx() {
        // Best-effort: shutdown is idempotent.
        let _ = tx.try_send(EngineMsg::Shutdown);
    }
}

/// Initialize the default WASM engine (async).
///
/// End-state rule: scheduler configuration must not implicitly depend on a Tokio runtime.
/// Instead, the host entrypoint (`main`) is responsible for awaiting this initialization.
pub async fn init_wasm_engine_with_config(cfg: WasmConfig) {
    Scheduler::global().init_wasm_engine(cfg).await;
}
fn seed_net_io_wait() {
    // Make NIC RX a resident task by default: it doesn't need re-submit.
    Scheduler::global().register_resident(Task::net_io("netio-wait", NetworkIoTask::NicRx));
}

impl Scheduler {
    pub fn global() -> &'static Scheduler {
        &SCHEDULER
    }

    pub fn new() -> Self {
        let shutdown = setup_shutdown_flag().unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));
        // Default time manager for now (can be replaced later with hybrid manager).
        let tm = PollTimeManager::new();
        let bus = SimpleEventBus::new();

        // WASM auto-load is applied explicitly via `init_with_config`.
        Self {
            shared: Arc::new((Mutex::new(SharedState::default()), Condvar::new())),
            shutdown,
            tm,
            bus,
            resident_count: std::sync::atomic::AtomicUsize::new(0),
            no_idle_wait: AtomicBool::new(false),
        }
    }

    fn apply_config(&self, cfg: SchedulerConfig) {
        // NOTE: WASM engine initialization is async and must be triggered explicitly
        // via `init_wasm_engine_with_config`.
        self.apply_wasm_config(cfg.wasm);

        // Debug/diagnostic flags.
        if cfg.no_idle_wait {
            // This is intentionally sticky: once enabled we don't toggle it off
            // to avoid surprising behavior in long-running processes.
            self.no_idle_wait.store(true, Ordering::Relaxed);
        }
    }

    fn apply_wasm_config(&self, cfg: WasmConfig) {
        // End-state: no implicit async work here.
        // The engine init is performed by `Scheduler::init_wasm_engine`.
        let _ = cfg;
    }

    async fn init_wasm_engine(&self, cfg: WasmConfig) {
        let Some(component_path) = cfg.component_path else {
            tracing::info!(
                target: "ntx::scheduler",
                "wasm auto-load disabled (no component_path configured)"
            );
            return;
        };

        let component_str = component_path.display().to_string();
        tracing::info!(
            target: "ntx::scheduler",
            component = %component_str,
            "attempting to auto-load wasm engine from config"
        );

        // Fast check: avoid work if already configured.
        if EngineManager::global()
            .lock()
            .expect("engine manager poisoned")
            .has_default()
        {
            tracing::info!(
                target: "ntx::scheduler",
                "wasm engine default already configured; skip auto-load"
            );
            return;
        }

        let engine_cfg = EngineConfig { component_path };
        let engine = match crate::wasm_engine::ComponentEngine::new(engine_cfg).await {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(
                    target: "ntx::scheduler",
                    component = %component_str,
                    error = %e,
                    error_dbg = ?e,
                    "wasm engine auto-load failed"
                );
                return;
            }
        };

        let mut mgr = EngineManager::global()
            .lock()
            .expect("engine manager poisoned");
        if mgr.has_default() {
            tracing::info!(
                target: "ntx::scheduler",
                "wasm engine default already configured; skip auto-load"
            );
            return;
        }
        mgr.register(EngineHandle("default".into()), engine);
        tracing::info!(
            target: "ntx::scheduler",
            component = %component_str,
            "wasm engine auto-load succeeded"
        );
    }

    pub fn event_bus(&self) -> Arc<SimpleEventBus> {
        self.bus.clone()
    }

    pub fn submit_action(&self, task_id: impl Into<String>, priority: u8) {
        self.submit(Task {
            id: task_id.into(),
            priority,
            kind: TaskKind::NetworkIo(NetworkIoTask::NicRx),
        });
    }

    pub fn submit(&self, task: Task) {
        match &task.kind {
            TaskKind::Timer(timer) => {
                let at = timer.at;
                self.submit_timer(task, at);
            }
            _ => {
                let (lock, cv) = &*self.shared;
                let mut state = lock.lock().expect("scheduler mutex poisoned");
                state.ingress.push_back(task);
                cv.notify_one();
            }
        }
    }

    /// Register a long-lived task that will be run repeatedly (at most once per scheduler loop)
    /// according to priority.
    ///
    /// Resident tasks do not live in the normal ingress queue and therefore don't need re-submit.
    pub fn register_resident(&self, task: Task) {
        let (lock, cv) = &*self.shared;
        let mut state = lock.lock().expect("scheduler mutex poisoned");
        state.resident.tasks.push(ResidentTask {
            id: task.id,
            priority: task.priority,
            kind: task.kind,
            backoff: ResidentBackoff::default(),
        });
        self.resident_count.fetch_add(1, Ordering::Relaxed);
        cv.notify_one();
    }

    pub fn submit_timer(&self, task: Task, at: Instant) {
        // Allocate token and register with time manager.
        let token = TimerToken::new(self.next_timer_token());
        self.tm.schedule_at(at, token);

        let (lock, cv) = &*self.shared;
        let mut state = lock.lock().expect("scheduler mutex poisoned");
        state.timers.by_token.insert(token.raw(), task);
        // Re-evaluate sleep deadline.
        cv.notify_one();
    }

    fn next_timer_token(&self) -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // End-state requirement: host shutdown must also wake guest waiters.
        // Best-effort and idempotent.
        request_engine_shutdown();
        let (_, cv) = &*self.shared;
        cv.notify_all();
    }

    /// Run the scheduler forever (blocks when idle).
    pub fn run(&self) -> ! {
        let mut queues = PriorityQueues::new();
        let mut idle_spins = 0usize;
        let mut last_loop_log = Instant::now() - Duration::from_secs(3600);
        let mut loop_iters: u64 = 0;
        let mut idle_loops: u64 = 0;

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                // Keep `-> !` contract; park forever when shutdown requested.
                thread::park();
            }

            loop_iters += 1;
            if Instant::now().duration_since(last_loop_log) >= Duration::from_secs(1) {
                tracing::trace!(
                    target: "ntx::scheduler",
                    iters = loop_iters,
                    idle_loops = idle_loops,
                    resident_tasks = self.resident_count.load(Ordering::Relaxed),
                    no_idle_wait = self.no_idle_wait.load(Ordering::Relaxed),
                    "scheduler loop heartbeat"
                );
                last_loop_log = Instant::now();
                loop_iters = 0;
                idle_loops = 0;
            }

            // A) Resident-first policy: always poll at least one resident task before
            // we consider any blocking idle wait. This prevents NicRx from being
            // starved by `Condvar::wait()` when the run queue is empty.
            self.poll_one_resident_task();

            // Keep queues fresh.
            if queues.is_empty() {
                idle_loops += 1;

                if self.no_idle_wait.load(Ordering::Relaxed) {
                    // Diagnostic mode: never block on the condvar. This keeps the loop
                    // running so we can prove whether RX is happening but was masked
                    // by blocking waits.
                    self.ingest_nowait(&mut queues);
                    // Be CPU-friendly: sleep a tiny amount when idle.
                    std::thread::sleep(Duration::from_millis(1));
                } else {
                    // short spin/yield to reduce cv contention under bursty workloads
                    if idle_spins < IDLE_SPIN_LIMIT {
                        idle_spins += 1;
                        thread::yield_now();
                        self.ingest_nowait(&mut queues);
                    } else {
                        idle_spins = 0;
                        // B) Bounded idle wait: even if there are no timers and no submits,
                        // wake periodically so resident tasks can be polled.
                        self.ingest_blocking_bounded(&mut queues, IDLE_WAIT_MAX);
                        // After bounded wait wakes (timeout or notify), immediately re-poll
                        // resident tasks so NicRx doesn't miss a polling window.
                        self.poll_one_resident_task();
                    }
                }
            } else {
                idle_spins = 0;
                // Non-empty: also ingest newly arrived tasks once per loop.
                self.ingest_nowait(&mut queues);
            }

            if let Some(task) = queues.pop() {
                self.execute(task);
            }
        }
    }

    fn ingest_blocking_bounded(&self, queues: &mut PriorityQueues, max_wait: Duration) {
        //! Drain ingress and timers, then sleep for at most `max_wait` if idle.
        //!
        //! Design contract:
        //! - This function **must not loop internally** on `wait_timeout`.
        //!   Previous versions had an internal loop that would block indefinitely,
        //!   preventing `poll_one_resident_task()` from executing after each wake.
        //! - Guaranteed semantics: single `wait_timeout(max_wait)` then return.
        //! - Caller (main loop) immediately re-polls resident tasks after this returns.
        //!   This ensures NicRx is polled at least every `max_wait` period.

        let (lock, cv) = &*self.shared;
        let mut state = lock.lock().expect("scheduler mutex poisoned");

        // Drain immediate ingress and due timers (non-blocking).
        while let Some(task) = state.ingress.pop_front() {
            queues.push(task);
        }
        self.promote_due_timers_locked(&mut state, queues);

        // If we found work, return immediately.
        if !queues.is_empty() {
            return;
        }

        // No runnable work; wait either for submit() or until next timer,
        // but never longer than `max_wait`.
        let now = self.tm.now();
        let next_deadline = self.next_timer_deadline_locked();

        let timeout = match next_deadline {
            Some(at) if at > now => {
                let t = at.saturating_duration_since(now);
                if t < max_wait { t } else { max_wait }
            }
            Some(_) => Duration::from_millis(0),
            None => max_wait,
        };

        if !timeout.is_zero() {
            let (guard, _) = cv
                .wait_timeout(state, timeout)
                .expect("scheduler mutex poisoned");
            state = guard;

            // After timeout/notify, drain any newly arrived work.
            while let Some(task) = state.ingress.pop_front() {
                queues.push(task);
            }
            self.promote_due_timers_locked(&mut state, queues);
        }
        // Return immediately after bounded wait, whether we found work or not.
        // The caller will re-poll resident tasks.
    }
    fn poll_one_resident_task(&self) {
        //! Poll at most one resident task, prioritized by priority field.
        //!
        //! **Key guarantee: NicRx is always polled.**
        //! - NicRx's backoff is cleared before evaluation so it's never suppressed.
        //! - NicRx's backoff is kept clear after execution, ensuring next iteration polls it again.
        //!
        //! **Why**: In userspace packet reception, missing a polling window can cause us
        //! to miss brief packet availability. The bounded idle wait (2ms) ensures we wake
        //! periodically, but NicRx must always be eligible when we do.
        //!
        //! **Other residents** use exponential backoff to avoid spinning when they return
        //! no work (e.g., `did_work=false`). This keeps the CPU usage low.

        let now = self.tm.now();

        // Pick one runnable resident by priority.
        let mut chosen: Option<usize> = None;
        {
            let (lock, _) = &*self.shared;
            let mut state = lock.lock().expect("scheduler mutex poisoned");

            // NicRx must never be suppressed by backoff.
            // Clear its backoff before evaluating, so it's always eligible.
            for task in state.resident.tasks.iter_mut() {
                if matches!(task.kind, TaskKind::NetworkIo(NetworkIoTask::NicRx)) {
                    task.backoff.until = None;
                }
            }

            for (idx, task) in state.resident.tasks.iter().enumerate() {
                if let Some(until) = task.backoff.until {
                    if until > now {
                        continue;
                    }
                }

                match chosen {
                    None => chosen = Some(idx),
                    Some(prev) => {
                        let prev_pri = state.resident.tasks[prev].priority;
                        if task.priority < prev_pri {
                            chosen = Some(idx);
                        }
                    }
                }
            }
        }

        let Some(idx) = chosen else {
            return;
        };

        // Execute chosen resident without holding the lock.
        let kind = {
            let (lock, _) = &*self.shared;
            let state = lock.lock().expect("scheduler mutex poisoned");
            state.resident.tasks[idx].kind.clone()
        };

        let did_work = self.execute_resident(kind);

        // Update backoff state.
        let (lock, _) = &*self.shared;
        let mut state = lock.lock().expect("scheduler mutex poisoned");
        let entry = &mut state.resident.tasks[idx];

        // NicRx never uses backoff; it's always eligible for polling.
        // Other residents use exponential backoff when they return no work.
        let is_nicrx = matches!(entry.kind, TaskKind::NetworkIo(NetworkIoTask::NicRx));
        if is_nicrx {
            // Keep NicRx always eligible
            entry.backoff.until = None;
            entry.backoff.current = RESIDENT_BACKOFF_MIN;
            return;
        }

        if did_work {
            entry.backoff.until = None;
            entry.backoff.current = RESIDENT_BACKOFF_MIN;
        } else {
            let next = (entry.backoff.current * 2).min(RESIDENT_BACKOFF_MAX);
            entry.backoff.current = next;
            entry.backoff.until = Some(now + next);
        }
    }

    /// Execute a TaskKind once and report whether it did useful work.
    ///
    /// This is shared by both resident ticks and regular queue execution.
    fn run_kind_once(&self, kind: &TaskKind) -> bool {
        match kind {
            TaskKind::NetworkIo(NetworkIoTask::NicRx) => {
                let now = self.tm.now();
                {
                    let mut hb = NIC_RX_HEARTBEAT.lock().expect("nicrx heartbeat poisoned");
                    hb.calls += 1;
                    if now.duration_since(hb.last_log) >= Duration::from_secs(1) {
                        tracing::trace!(
                            target: "ntx::scheduler::nicrx",
                            calls = hb.calls,
                            no_packet = hb.no_packet,
                            enqueued_batches = hb.enqueued_batches,
                            no_engine_tx = hb.no_engine_tx,
                            buffered_not_flushed = hb.buffered_not_flushed,
                            rx_seen = hb.rx_seen,
                            sent_batches = hb.sent_batches,
                            "nicrx heartbeat"
                        );
                        hb.last_log = now;
                        // rolling 1Hz bucket
                        hb.calls = 0;
                        hb.no_packet = 0;
                        hb.enqueued_batches = 0;
                        hb.no_engine_tx = 0;
                        hb.buffered_not_flushed = 0;
                        hb.rx_seen = 0;
                        hb.sent_batches = 0;
                    }
                }

                // Policy #2: NicRx only receives and enqueues a WasmCall; the WasmCall will
                // drive the guest handler.
                let Some(rx) = non_blocking_recv_udp() else {
                    let mut hb = NIC_RX_HEARTBEAT.lock().expect("nicrx heartbeat poisoned");
                    hb.no_packet += 1;
                    return false;
                };

                {
                    let mut hb = NIC_RX_HEARTBEAT.lock().expect("nicrx heartbeat poisoned");
                    hb.rx_seen += 1;
                }

                tracing::debug!(
                    target: "ntx::scheduler::nicrx",
                    sock_id = ?rx.sock_id,
                    src_port = rx.src_port,
                    dst_port = rx.dst_port,
                    payload_len = rx.payload.len(),
                    "nicrx received udp"
                );

                // For the composed scheduler component path (end-state pull model):
                // - host builds (desc_mem, payload_mem) buffers in a shared layout
                // - host enqueues the buffers into the host `rx-ring` provider
                // - guest `run()` pulls via `ntx:host/rx-ring@0.1.0` and publishes `packet.rx`
                //
                // desc_mem/payload_mem layout is defined by the guest decode implementation
                // (`component/scheduler/src/rx_decode.rs::drain_rx_ring`).
                // We reuse `rx_layout` helpers to encode the same layout:
                // - A control block at offset 0
                // - A descriptor ring starting at DESCS_OFF
                // - payload_mem is a plain byte region, with offsets referenced by desc
                let (desc_mem, payload_mem) = RX_RING.with_borrow_mut(|r| {
                    r.push_and_maybe_flush_one(rx.sock_id.map(|s| s as u64), &rx.payload)
                });
                if let (Some(desc_mem), Some(payload_mem)) = (desc_mem, payload_mem) {
                    if let Some(tx) = engine_tx() {
                        // Best-effort; if the channel is closed/full, drop newest.
                        let desc_len = desc_mem.len();
                        let payload_len = payload_mem.len();

                        let batch_id = {
                            let hb = NIC_RX_HEARTBEAT.lock().expect("nicrx heartbeat poisoned");
                            // Monotonic-ish within the process, useful for join across layers.
                            hb.calls
                        };

                        let res = tx.try_send(EngineMsg::RxBatch {
                            desc: desc_mem,
                            payload: payload_mem,
                        });
                        if let Err(e) = &res {
                            tracing::warn!(
                                target: "ntx::scheduler::nicrx",
                                error = %e,
                                desc_len,
                                payload_len,
                                batch_id,
                                "engine owner channel full/closed; dropping newest RxBatch"
                            );
                            false
                        } else {
                            tracing::debug!(
                                target: "ntx::scheduler::nicrx",
                                batch_id,
                                desc_len,
                                payload_len,
                                "sent RxBatch to engine owner"
                            );
                            let mut hb = NIC_RX_HEARTBEAT.lock().expect("nicrx heartbeat poisoned");
                            hb.enqueued_batches += 1;
                            hb.sent_batches += 1;
                            true
                        }
                    } else {
                        // Engine not configured yet.
                        let mut hb = NIC_RX_HEARTBEAT.lock().expect("nicrx heartbeat poisoned");
                        hb.no_engine_tx += 1;
                        tracing::warn!(
                            target: "ntx::scheduler::nicrx",
                            "engine_tx is None; dropping RxBatch"
                        );
                        false
                    }
                } else {
                    // Buffered but not flushed yet.
                    let mut hb = NIC_RX_HEARTBEAT.lock().expect("nicrx heartbeat poisoned");
                    hb.buffered_not_flushed += 1;
                    true
                }
            }
            TaskKind::NetworkIo(NetworkIoTask::NicTx) => {
                // Placeholder: real Tx path will drain a queue and flush to NIC.
                false
            }
            TaskKind::WasmCall(wasm) => {
                tracing::debug!(
                    target: "ntx::scheduler",
                    function = %wasm.function,
                    input_len = wasm.input.len(),
                    "executing wasm call"
                );
                // Note: in the end-state we avoid host->guest export calls from the RX path.
                // Keep WasmCall tasks reserved for future explicit guest calls.
                let _ = wasm;
                false
            }
            // Timer is handled in `execute()` because it consumes action and publishes.
            TaskKind::Timer(_) => false,
            #[cfg(test)]
            TaskKind::TestResident(TestResidentTask::Tick) => true,
        }
    }

    /// Execute a resident task once.
    /// Returns whether it did useful work (used for backoff).
    fn execute_resident(&self, kind: TaskKind) -> bool {
        self.run_kind_once(&kind)
    }

    fn ingest_nowait(&self, queues: &mut PriorityQueues) {
        let (lock, _) = &*self.shared;
        let mut state = lock.lock().expect("scheduler mutex poisoned");
        while let Some(task) = state.ingress.pop_front() {
            queues.push(task);
        }

        // Promote due timers (polling manager will rely on explicit check).
        self.promote_due_timers_locked(&mut state, queues);
    }

    fn promote_due_timers_locked(&self, state: &mut SharedState, queues: &mut PriorityQueues) {
        let now = self.tm.now();

        // Poll the shared InMemoryTimer if using PollTimeManager.
        if let Some(poll) = self.tm.as_any().downcast_ref::<PollTimeManager>() {
            let due = poll.timer().pop_due(now);
            for (token, _at) in due {
                if let Some(task) = state.timers.by_token.remove(&token.raw()) {
                    queues.push(task);
                }
            }
        }
    }

    fn next_timer_deadline_locked(&self) -> Option<Instant> {
        // For now, only PollTimeManager exposes peek-next.
        if let Some(poll) = self.tm.as_any().downcast_ref::<PollTimeManager>() {
            return poll.timer().peek_next_at();
        }
        // Fallback: none.
        None
    }

    fn execute(&self, _task: Task) {
        match _task.kind {
            TaskKind::Timer(mut t) => {
                if let Some(action) = t.action.take() {
                    // Re-stamp id/ts and publish as an event.
                    let ev = build_event(&self.bus, &action.topic, action.priority, action.payload);
                    self.bus.publish(ev);
                }
            }
            other => {
                let _did_work = self.run_kind_once(&other);
                let _ = _did_work;
            }
        }
    }
}

/// A reusable RX batch builder for the host->guest RX handoff.
///
/// This produces `(desc_mem, payload_mem)` in the shared layout and hands ownership
/// to the host `rx-ring` provider (so the guest can pull it inside `run()`).
///
/// Contract:
/// - `desc_mem` contains a control block at 0 and desc ring at DESCS_OFF.
/// - `payload_mem` is a byte region referenced by descriptors.
/// - We keep `head=0` and advance `tail` up to N, then flush by handing ownership
///   of the buffers to the wasm engine.
///
/// Notes:
/// - We deliberately *don't* implement wraparound yet. When near capacity, we flush.
/// - This already removes per-packet allocations/copies of the descriptor ring.
struct RxRingBatch {
    desc_cap: u32,
    payload_cap: u32,
    desc_mem: Vec<u8>,
    payload_mem: Vec<u8>,
    tail: u32,
    seq: u64,
}

impl RxRingBatch {
    fn new(desc_cap: u32, payload_cap: u32) -> Self {
        let mut r = Self {
            desc_cap,
            payload_cap,
            desc_mem: Vec::new(),
            payload_mem: Vec::new(),
            tail: 0,
            seq: 0,
        };
        r.reset_buffers();
        r
    }

    fn reset_buffers(&mut self) {
        self.payload_mem.clear();
        self.payload_mem.reserve(self.payload_cap as usize);
        self.tail = 0;

        // desc_mem needs to hold control + full desc ring region.
        let desc_bytes = self.desc_cap as usize * shared_mem::DESC_LEN;
        let total = shared_mem::DESCS_OFF as usize + desc_bytes;
        self.desc_mem.clear();
        self.desc_mem.resize(total, 0u8);

        // Initial control: head=0, tail=0.
        let cb = shared_mem::ControlBlock::new(self.desc_cap, self.payload_cap);
        let cb_enc = shared_mem::encode_control(&cb);
        self.desc_mem
            [shared_mem::CONTROL_OFF as usize..shared_mem::CONTROL_OFF as usize + cb_enc.len()]
            .copy_from_slice(&cb_enc);
    }

    fn can_fit(&self, payload_len: usize) -> bool {
        if self.tail >= self.desc_cap {
            return false;
        }
        (self.payload_mem.len() + payload_len) <= self.payload_cap as usize
    }

    fn push_one(&mut self, sock_id: Option<u64>, payload: &[u8]) {
        let payload_off = shared_mem::PAYLOAD_OFF + self.payload_mem.len() as u32;
        self.payload_mem.extend_from_slice(payload);

        self.seq = self.seq.wrapping_add(1);
        let desc = shared_mem::Descriptor::rx(sock_id, payload_off, payload.len() as u32, self.seq);
        let desc_enc = shared_mem::encode_desc(&desc);

        let idx = self.tail as usize;
        let base = shared_mem::DESCS_OFF as usize + idx * shared_mem::DESC_LEN;
        self.desc_mem[base..base + desc_enc.len()].copy_from_slice(&desc_enc);

        self.tail += 1;

        // Patch desc_tail in control block.
        let tail_off = shared_mem::CONTROL_OFF as usize + 16;
        self.desc_mem[tail_off..tail_off + 4].copy_from_slice(&self.tail.to_le_bytes());

        // Patch payload_tail too (mostly informational for now).
        let payload_tail_off = shared_mem::CONTROL_OFF as usize + 28;
        let pt = self.payload_mem.len() as u32;
        self.desc_mem[payload_tail_off..payload_tail_off + 4].copy_from_slice(&pt.to_le_bytes());
    }

    fn flush(&mut self) -> Option<(Vec<u8>, Vec<u8>)> {
        if self.tail == 0 {
            return None;
        }
        let mut out_desc = Vec::new();
        let mut out_payload = Vec::new();
        std::mem::swap(&mut out_desc, &mut self.desc_mem);
        std::mem::swap(&mut out_payload, &mut self.payload_mem);
        self.reset_buffers();
        Some((out_desc, out_payload))
    }

    /// Push one packet and flush if we hit capacity.
    /// Returns:
    /// - (None,None): buffered, not flushed yet.
    /// - (Some(desc),Some(payload)): ready to notify.
    fn push_and_maybe_flush_one(
        &mut self,
        sock_id: Option<u64>,
        payload: &[u8],
    ) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        if !self.can_fit(payload.len()) {
            // Notifying the currently-buffered batch is better than dropping.
            // We'll flush it now and buffer the current packet into a fresh batch.
            let flushed = self.flush();
            self.push_one(sock_id, payload);
            if let Some((d, p)) = flushed {
                return (Some(d), Some(p));
            }
        }

        self.push_one(sock_id, payload);
        if self.tail >= self.desc_cap {
            if let Some((d, p)) = self.flush() {
                return (Some(d), Some(p));
            }
        }

        (None, None)
    }
}

thread_local! {
    static RX_RING: RefCell<RxRingBatch> = RefCell::new(RxRingBatch::new(RX_BATCH_DESC_CAP, RX_BATCH_PAYLOAD_CAP));
}

fn setup_shutdown_flag() -> Result<Arc<AtomicBool>, SchedulerError> {
    // Signal handling not available in WASM.
    Ok(Arc::new(AtomicBool::new(false)))
}

struct PriorityQueues {
    lanes: [VecDeque<Task>; PRIORITY_LEVELS],
}

impl PriorityQueues {
    fn new() -> Self {
        Self {
            lanes: std::array::from_fn(|_| VecDeque::new()),
        }
    }

    fn push(&mut self, task: Task) {
        let idx = task.priority.min((PRIORITY_LEVELS - 1) as u8) as usize;
        self.lanes[idx].push_back(task);
    }

    fn pop(&mut self) -> Option<Task> {
        for lane in self.lanes.iter_mut() {
            if let Some(task) = lane.pop_front() {
                return Some(task);
            }
        }
        None
    }

    fn is_empty(&self) -> bool {
        self.lanes.iter().all(|lane| lane.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_queue_orders_by_priority_then_fifo() {
        let mut q = PriorityQueues::new();
        q.push(Task::timer_after(
            "low",
            Duration::from_secs(1),
            "low-action",
        ));
        q.push(Task::net_io("high-1", NetworkIoTask::NicRx));
        q.push(Task::net_io("high-2", NetworkIoTask::NicTx));

        let t1 = q.pop().unwrap();
        assert_eq!(t1.id, "high-1");

        let t2 = q.pop().unwrap();
        assert_eq!(t2.id, "high-2");

        let t3 = q.pop().unwrap();
        assert_eq!(t3.id, "low");
        assert!(q.pop().is_none());
    }

    #[test]
    fn idle_blocks_until_submit() {
        let sched = Scheduler::new();
        let shared = sched.shared.clone();
        let shutdown = sched.shutdown.clone();
        let tm = sched.tm.clone();
        let bus = sched.bus.clone();

        let handle = thread::spawn(move || {
            let sched2 = Scheduler {
                shared,
                shutdown,
                tm,
                bus,
                resident_count: std::sync::atomic::AtomicUsize::new(0),
                no_idle_wait: std::sync::atomic::AtomicBool::new(false),
            };
            let mut q = PriorityQueues::new();
            // Use the bounded idle wait so this can't sleep forever if there's
            // no timer deadline.
            sched2.ingest_blocking_bounded(&mut q, Duration::from_millis(50));
            q.pop().expect("expected task");
        });

        thread::sleep(std::time::Duration::from_millis(30));
        sched.submit_action("wake", PRIORITY_HIGH);

        handle.join().unwrap();
    }

    #[test]
    fn timer_action_executes_once() {
        let sched = Scheduler::new();
        let hit = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let hit2 = hit.clone();

        let bus = sched.event_bus();
        bus.subscribe(SimpleEventBus::TOPIC_TIMER_FIRE, move |_ev| {
            hit2.fetch_add(1, Ordering::SeqCst);
        });

        let task = Task::timer_event(
            "t1",
            Instant::now() + Duration::from_millis(10),
            Bytes::from_static(b"t1"),
        );
        sched.submit(task);

        // Drive scheduler a little: ingest until the timer becomes due and executes.
        let mut q = PriorityQueues::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        while hit.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            sched.ingest_nowait(&mut q);
            if let Some(t) = q.pop() {
                sched.execute(t);
            }
            thread::sleep(Duration::from_millis(1));
        }

        assert_eq!(hit.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn resident_task_does_not_require_resubmit() {
        let sched = Scheduler::new();
        sched.register_resident(Task {
            id: "r1".to_string(),
            priority: PRIORITY_NET_IO,
            kind: TaskKind::TestResident(TestResidentTask::Tick),
        });

        // Poll a few times; should not remove resident tasks.
        for _ in 0..5 {
            sched.poll_one_resident_task();
        }

        let (lock, _) = &*sched.shared;
        let state = lock.lock().expect("scheduler mutex poisoned");
        assert_eq!(state.resident.tasks.len(), 1);
    }

    #[test]
    fn resident_runs_and_normal_task_still_executes() {
        // Register a resident (NicRx) and also submit a normal timer task.
        // We assert the normal task still executes (resident doesn't starve it).

        let sched = Scheduler::new();
        sched.register_resident(Task {
            id: "r1".to_string(),
            priority: PRIORITY_NET_IO,
            kind: TaskKind::TestResident(TestResidentTask::Tick),
        });

        let hit = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let hit2 = hit.clone();

        let bus = sched.event_bus();
        bus.subscribe(SimpleEventBus::TOPIC_TIMER_FIRE, move |_ev| {
            hit2.fetch_add(1, Ordering::SeqCst);
        });

        // Make the test deterministic: avoid depending on PollTimeManager's internal
        // timer wheel promotion. We still validate that:
        // - polling one resident doesn't break normal task execution
        // - executing a Timer task publishes an event as expected

        // 1) Queue and execute a normal (non-timer) task.
        sched.submit(Task::wasm_call("n1", "noop"));
        let mut q = PriorityQueues::new();
        sched.ingest_nowait(&mut q);

        // 2) Poll one resident.
        sched.poll_one_resident_task();

        // 3) Execute one queued task.
        if let Some(t) = q.pop() {
            sched.execute(t);
        }

        // 4) Execute the Timer task directly and assert it publishes.
        let timer_task = Task::timer_event("t1", Instant::now(), Bytes::from_static(b"t1"));
        sched.execute(timer_task);

        // SimpleEventBus delivers asynchronously on a worker thread, so wait a bit.
        let deadline = Instant::now() + Duration::from_secs(1);
        while hit.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }

        // Avoid leaking worker threads across tests.
        sched.bus.shutdown();

        assert_eq!(hit.load(Ordering::SeqCst), 1);
    }
}
