//! Load controller (ramp-up users)
//!
//! Maintains a small state machine that periodically publishes `scheduler.user.start`
//! based on scenario.ramp_up.phases.

use once_cell::sync::Lazy;
use std::sync::Mutex;

use crate::eventing::events::EVENT_COUNTER;
use crate::scenario::scenario_types::RampPhase;

#[derive(Clone, Debug, Default)]
pub(crate) struct LoadControllerState {
    pub(crate) started_at_ms: u64,
    pub(crate) next_phase: usize,
    pub(crate) next_user_seq: u64,
    // cached phases
    pub(crate) phases: Vec<RampPhase>,
}

pub(crate) static LOAD: Lazy<Mutex<LoadControllerState>> =
    Lazy::new(|| Mutex::new(LoadControllerState::default()));

pub(crate) fn tick_load_controller(now_ms: u64) {
    let (phases, started_at_ms, next_phase, next_user_seq) = {
        if let Ok(lc) = LOAD.lock() {
            (
                lc.phases.clone(),
                lc.started_at_ms,
                lc.next_phase,
                lc.next_user_seq,
            )
        } else {
            return;
        }
    };
    if phases.is_empty() {
        return;
    }

    let elapsed_sec = now_ms.saturating_sub(started_at_ms) / 1000;
    let mut idx = next_phase;
    let mut seq = next_user_seq;

    while idx < phases.len() && phases[idx].at_second <= elapsed_sec {
        let spawn = phases[idx].spawn_users;
        publish_user_start_event(now_ms, spawn as u64, Some(seq));
        seq += spawn as u64;
        idx += 1;
    }

    if let Ok(mut lc) = LOAD.lock() {
        lc.next_phase = idx;
        lc.next_user_seq = seq;
    }
}

pub(crate) fn publish_user_start_event(now_ms: u64, spawn_users: u64, start_seq: Option<u64>) {
    let base = start_seq.unwrap_or(1);
    for i in 0..spawn_users {
        let user_id = format!("user-{}", base + i);
        let payload = serde_json::to_string(&crate::UserStartPayload {
            user_id: user_id.clone(),
        })
        .unwrap_or_else(|_| "{}".to_string());
        let id = format!(
            "us-{}",
            EVENT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let _ = crate::ntx::scenario_eventbus::event_bus::publish(
            &crate::ntx::scenario_eventbus::event_bus::Event {
                id,
                kind: crate::EventKind::SchedulerUserStart.as_str().to_string(),
                // IMPORTANT: downstream scheduler logic often keys off `ev.user_id`.
                // Keep `payload.user_id` for compatibility, but always set the structured field.
                user_id: Some(user_id),
                task_id: None,
                action_id: None,
                payload,
                correlation_id: None,
                timestamp_ms: now_ms,
            },
        );
    }
}
