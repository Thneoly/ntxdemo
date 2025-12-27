//! Timers: in-memory timer wheel (very small / MVP)

use once_cell::sync::Lazy;
use std::sync::Mutex;

use crate::events::EVENT_COUNTER;

#[derive(Clone, Debug)]
pub(crate) struct TimerJob {
    pub(crate) due_ms: u64,
    pub(crate) kind: String, // event kind, e.g. scheduler.timer.timeout / scheduler.timer.retry
    pub(crate) user_id: Option<String>,
    pub(crate) task_id: Option<String>,
    pub(crate) action_id: Option<String>,
    pub(crate) payload: String, // json string
}

pub(crate) static TIMERS: Lazy<Mutex<Vec<TimerJob>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub(crate) fn tick_timers(now_ms: u64) {
    let mut due: Vec<TimerJob> = Vec::new();
    if let Ok(mut timers) = TIMERS.lock() {
        let mut i = 0usize;
        while i < timers.len() {
            if timers[i].due_ms <= now_ms {
                due.push(timers.remove(i));
            } else {
                i += 1;
            }
        }
    }

    for t in due {
        let id = format!(
            "tm-{}",
            EVENT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let _ = crate::ntx::scenario_eventbus::event_bus::publish(
            &crate::ntx::scenario_eventbus::event_bus::Event {
                id,
                kind: t.kind,
                user_id: t.user_id,
                task_id: t.task_id,
                action_id: t.action_id,
                payload: t.payload,
                correlation_id: None,
                timestamp_ms: now_ms,
            },
        );
    }
}

pub(crate) fn schedule_timer(
    kind: &str,
    due_ms: u64,
    user_id: &str,
    task_id: &str,
    action_id: Option<&str>,
    payload: serde_json::Value,
) {
    let job = TimerJob {
        due_ms,
        kind: kind.to_string(),
        user_id: Some(user_id.to_string()),
        task_id: Some(task_id.to_string()),
        action_id: action_id.map(|s| s.to_string()),
        payload: payload.to_string(),
    };
    if let Ok(mut timers) = TIMERS.lock() {
        timers.push(job);
    }
}
