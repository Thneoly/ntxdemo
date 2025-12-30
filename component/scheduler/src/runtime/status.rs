use once_cell::sync::Lazy;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use crate::eventing::events::EVENT_COUNTER;
use crate::now_ms;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SchedulerState {
    Idle,
    Running,
    Completed,
    Error,
}

static SCHED_STATE: Lazy<Mutex<SchedulerState>> = Lazy::new(|| Mutex::new(SchedulerState::Idle));

pub(crate) fn publish_scheduler_state(state: SchedulerState, err: Option<&String>) {
    if let Ok(mut st) = SCHED_STATE.lock() {
        *st = state;
    }

    let payload = serde_json::to_string(&crate::SchedulerStateChangedPayload {
        state: format!("{:?}", state),
        error: err.cloned(),
    })
    .unwrap_or_else(|_| "{}".to_string());

    let id = format!("ss-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed));
    let _ = crate::ntx::scenario_eventbus::event_bus::publish(
        &crate::ntx::scenario_eventbus::event_bus::Event {
            id,
            kind: crate::EventKind::SchedulerStateChanged.as_str().to_string(),
            user_id: None,
            task_id: None,
            action_id: None,
            payload,
            correlation_id: None,
            timestamp_ms: now_ms(),
        },
    );
}
