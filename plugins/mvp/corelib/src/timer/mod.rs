use crate::CoreLib;
use crate::exports::ntx::runner::core_timer::Guest;
use crate::exports::ntx::runner::core_timer::{GuestTimerHandle, TimerHandle, TimerHandleBorrow};
use crate::ntx::runner::types::{SchedulerError, SchedulerErrorKind};
use crate::wasi::clocks::{
    monotonic_clock::{self, Duration, Instant},
    wall_clock::{self, Datetime},
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Debug)]
pub struct ResTimerHandle {
    id: u64,
}

impl ResTimerHandle {
    fn new(id: u64) -> Self {
        Self { id }
    }

    fn id(&self) -> u64 {
        self.id
    }
}

#[derive(Debug, Clone)]
struct TimerEntry {
    token: String,
    deadline: Instant,
    created_at: Instant,
}

static TIMER_REGISTRY: OnceLock<Mutex<HashMap<u64, TimerEntry>>> = OnceLock::new();
static NEXT_TIMER_ID: AtomicU64 = AtomicU64::new(1);

fn registry() -> &'static Mutex<HashMap<u64, TimerEntry>> {
    TIMER_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_timer_id() -> u64 {
    NEXT_TIMER_ID.fetch_add(1, Ordering::Relaxed)
}

fn registry_error(message: impl Into<String>) -> SchedulerError {
    SchedulerError {
        kind: SchedulerErrorKind::Internal,
        message: message.into(),
    }
}

impl GuestTimerHandle for ResTimerHandle {}

impl Guest for CoreLib {
    type TimerHandle = ResTimerHandle;

    fn now() -> Instant {
        monotonic_clock::now()
    }
    fn utc_now() -> Datetime {
        wall_clock::now()
    }
    fn schedule_at(at: Instant, token: String) -> Result<TimerHandle, SchedulerError> {
        let id = next_timer_id();
        let entry = TimerEntry {
            token,
            deadline: at,
            created_at: monotonic_clock::now(),
        };

        let mut guard = registry()
            .lock()
            .map_err(|_| registry_error("timer registry poisoned"))?;
        guard.insert(id, entry);

        Ok(TimerHandle::new(ResTimerHandle::new(id)))
    }
    fn schedule_after(delay: Duration, token: String) -> Result<TimerHandle, SchedulerError> {
        let now = monotonic_clock::now();
        let deadline = now.saturating_add(delay);
        Self::schedule_at(deadline, token)
    }
    fn cancel(timer: TimerHandleBorrow<'_>) {
        let timer_id = timer.get::<ResTimerHandle>().id();
        if let Ok(mut guard) = registry().lock() {
            if let Some(entry) = guard.remove(&timer_id) {
                #[cfg(debug_assertions)]
                eprintln!(
                    "cancelled timer #{timer_id} token={} deadline={} created_at={}",
                    entry.token, entry.deadline, entry.created_at
                );
            }
        }
    }
}
