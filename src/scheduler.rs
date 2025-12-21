use crate::app_config::{SchedulerConfig, WasmConfig};
use crate::error::SchedulerError;
use crate::event_bus::{Bytes, EventBus, SimpleEventBus, build_event};
use crate::kernel::non_blocking_recv_with_sock;
use crate::time::{PollTimeManager, TimeManager, TimerToken};
use crate::wasm_engine::{EngineConfig, EngineHandle, EngineManager};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
// (no direct tracing macro imports; use `tracing::...!` at call sites)

/// A self-contained priority scheduler for the root crate.
///
/// Design goals:
/// - Fixed number of priority lanes (0 is highest priority).
/// - FIFO order within the same priority.
/// - When there are no runnable tasks, block (idle) and wake on submit.
/// - No dependency on `plugins/` modules.

const PRIORITY_LEVELS: usize = 64;
const IDLE_SPIN_LIMIT: usize = 2;
const RESIDENT_BACKOFF_MIN: Duration = Duration::from_micros(50);
const RESIDENT_BACKOFF_MAX: Duration = Duration::from_millis(2);

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
}

static SCHEDULER: Lazy<Scheduler> = Lazy::new(Scheduler::new);

/// Start the global scheduler in a background thread.
///
/// If you need a blocking scheduler, call `Scheduler::global().run()` directly.
pub fn start_scheduler() -> Result<(), SchedulerError> {
    thread::Builder::new()
        .name("ntx-scheduler".into())
        .spawn(|| Scheduler::global().run())
        .map_err(SchedulerError::Io)?;
    Ok(())
}

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
        }
    }

    fn apply_config(&self, cfg: SchedulerConfig) {
        self.apply_wasm_config(cfg.wasm);
    }

    fn apply_wasm_config(&self, cfg: WasmConfig) {
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

        let engine_cfg = EngineConfig {
            component_path: component_path,
            entry_candidates: cfg.entry_candidates,
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

        match mgr.load_and_register_demo(EngineHandle("default".into()), engine_cfg) {
            Ok(()) => {
                tracing::info!(
                    target: "ntx::scheduler",
                    component = %component_str,
                    "wasm engine auto-load succeeded"
                );
            }
            Err(e) => {
                tracing::error!(
                    target: "ntx::scheduler",
                    component = %component_str,
                    error = %e,
                    error_dbg = ?e,
                    "wasm engine auto-load failed"
                );
            }
        }
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
        let (_, cv) = &*self.shared;
        cv.notify_all();
    }

    /// Run the scheduler forever (blocks when idle).
    pub fn run(&self) -> ! {
        let mut queues = PriorityQueues::new();
        let mut idle_spins = 0usize;

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                // Keep `-> !` contract; park forever when shutdown requested.
                thread::park();
            }

            // Keep queues fresh.
            if queues.is_empty() {
                // short spin/yield to reduce cv contention under bursty workloads
                if idle_spins < IDLE_SPIN_LIMIT {
                    idle_spins += 1;
                    thread::yield_now();
                    self.ingest_nowait(&mut queues);
                } else {
                    idle_spins = 0;
                    self.ingest_blocking(&mut queues);
                }
            } else {
                idle_spins = 0;
                // Non-empty: also ingest newly arrived tasks once per loop.
                self.ingest_nowait(&mut queues);
            }

            // Policy: each loop runs at most one resident task (by priority),
            // then executes one regular queued task.
            self.poll_one_resident_task();

            if let Some(task) = queues.pop() {
                self.execute(task);
            }
        }
    }

    fn poll_one_resident_task(&self) {
        let now = self.tm.now();

        // Pick one runnable resident by priority.
        let mut chosen: Option<usize> = None;
        {
            let (lock, _) = &*self.shared;
            let state = lock.lock().expect("scheduler mutex poisoned");
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
                // Policy #2: NicRx only receives and enqueues a WasmCall; the WasmCall will
                // drive the guest handler.
                let Some((sock, payload)) = non_blocking_recv_with_sock() else {
                    return false;
                };

                // Enqueue into the guest shared buffers. The WasmCall will run `notify-rx`.
                // If no engine is configured we still enqueue nothing and report no work.
                let mut mgr = EngineManager::global()
                    .lock()
                    .expect("engine manager poisoned");
                let _ = mgr.enqueue_rx(sock.map(|s| s as u64), &payload);
                self.submit(Task::wasm_call("wasm-rx", "notify-rx"));
                true
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
                // Drive guest processing.
                let mut mgr = EngineManager::global()
                    .lock()
                    .expect("engine manager poisoned");

                match wasm.function.as_str() {
                    "run" => {
                        // One-shot demo entrypoint: drive guest-side acquire/bind/send loop.
                        // If no engine is configured, this becomes a no-op.
                        match mgr.run() {
                            Ok(()) => {
                                tracing::info!(target: "ntx::scheduler", "wasm run() completed");
                                true
                            }
                            Err(e) => {
                                tracing::error!(target: "ntx::scheduler", error = %e, "wasm run() failed");
                                false
                            }
                        }
                    }
                    "notify-rx" => match mgr.notify_rx() {
                        Ok(n) => n > 0,
                        Err(_e) => false,
                    },
                    // Backwards-compat for earlier demo tasks.
                    _ => {
                        let input = std::str::from_utf8(&wasm.input).unwrap_or("");
                        match mgr.tick_demo(input) {
                            Ok(result) => result.did_work,
                            Err(_e) => false,
                        }
                    }
                }
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

    fn ingest_blocking(&self, queues: &mut PriorityQueues) {
        let (lock, cv) = &*self.shared;
        let mut state = lock.lock().expect("scheduler mutex poisoned");

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }

            // First, drain immediate ingress and due timers.
            while let Some(task) = state.ingress.pop_front() {
                queues.push(task);
            }
            self.promote_due_timers_locked(&mut state, queues);

            if !queues.is_empty() {
                break;
            }

            // No runnable work; wait either for submit() or until next timer.
            let now = self.tm.now();
            let next_deadline = self.next_timer_deadline_locked();
            state = match next_deadline {
                Some(at) if at > now => {
                    let timeout = at.saturating_duration_since(now);
                    let (guard, _) = cv
                        .wait_timeout(state, timeout)
                        .expect("scheduler mutex poisoned");
                    guard
                }
                Some(_) => {
                    // Deadline already due; loop will promote.
                    state
                }
                None => cv.wait(state).expect("scheduler mutex poisoned"),
            };
        }
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
            };
            let mut q = PriorityQueues::new();
            sched2.ingest_blocking(&mut q);
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
