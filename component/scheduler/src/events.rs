//! Event publishing helpers.

use std::sync::atomic::{AtomicU64, Ordering};

/// Global event counter used to create unique event ids.
///
/// Note: kept in this module so other modules can publish without depending on `lib.rs` globals.
pub static EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[inline]
pub fn publish_event(
    kind: &str,
    user_id: Option<&str>,
    task_id: Option<&str>,
    action_id: Option<&str>,
    payload: serde_json::Value,
) {
    let _ = crate::ntx::scenario_eventbus::event_bus::publish(
        &crate::ntx::scenario_eventbus::event_bus::Event {
            id: format!("ev-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)),
            kind: kind.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            task_id: task_id.map(|s| s.to_string()),
            action_id: action_id.map(|s| s.to_string()),
            payload: payload.to_string(),
            correlation_id: None,
            timestamp_ms: crate::time::now_ms(),
        },
    );
}

#[inline]
pub fn publish_event_with_corr(
    kind: &str,
    user_id: Option<&str>,
    task_id: Option<&str>,
    action_id: Option<&str>,
    correlation_id: Option<&str>,
    payload: serde_json::Value,
) {
    let _ = crate::ntx::scenario_eventbus::event_bus::publish(
        &crate::ntx::scenario_eventbus::event_bus::Event {
            id: format!("ev-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)),
            kind: kind.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            task_id: task_id.map(|s| s.to_string()),
            action_id: action_id.map(|s| s.to_string()),
            payload: payload.to_string(),
            correlation_id: correlation_id.map(|s| s.to_string()),
            timestamp_ms: crate::time::now_ms(),
        },
    );
}
