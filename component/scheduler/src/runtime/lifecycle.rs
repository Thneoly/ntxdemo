//! User lifecycle helpers.
//!
//! Extracted from `lib.rs` to keep the crate entrypoint small.

use crate::eventing::events::EVENT_COUNTER;
use crate::eventing::payloads::UserExitPayload;
use crate::eventing::topics::EventKind;
use crate::runtime::runtime_state::RUNTIME;
use crate::scenario::scenario_registry::get_user_scenario_ctx;
use crate::scenario::scenario_types::Scenario;
use crate::scheduler::state_machine::SmEvent;
use crate::SchedulerContext;

/// Best-effort: if the state machine reached end, emit `scheduler.user.exit` once per iteration.
pub(crate) fn maybe_finish_user(_ctx: &SchedulerContext, user_id: &str) -> Result<(), String> {
    let (_ver, sc_arc, _wf_idx) = get_user_scenario_ctx(user_id)?;
    let sc = sc_arc.as_ref();
    let reached_end = {
        let sm = crate::STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        sm.is_end_reached(sc, user_id)
    };

    if reached_end {
        // publish only once per iteration
        let mut should_emit = false;
        if let Ok(mut rt) = RUNTIME.lock() {
            if let Some(u) = rt.users.get_mut(user_id) {
                if !u.meta.end_event_sent {
                    u.meta.end_event_sent = true;
                    should_emit = true;
                }
            }
        }
        if should_emit {
            publish_user_exit_event(user_id, "end-reached");
        }
    }

    Ok(())
}

pub(crate) fn publish_user_exit_event(user_id: &str, reason: &str) {
    let id = format!(
        "ux-{}",
        EVENT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let payload = serde_json::to_string(&UserExitPayload {
        user_id: user_id.to_string(),
        reason: reason.to_string(),
    })
    .unwrap_or_else(|_| "{}".to_string());
    let _ = crate::bindmod::ntx::scenario_eventbus::event_bus::publish(
        &crate::bindmod::ntx::scenario_eventbus::event_bus::Event {
            id,
            kind: EventKind::SchedulerUserExit.as_str().to_string(),
            user_id: Some(user_id.to_string()),
            task_id: None,
            action_id: None,
            payload,
            correlation_id: None,
            timestamp_ms: crate::now_ms(),
        },
    );
}

pub(crate) fn restart_user_iteration(_ctx: &SchedulerContext, user_id: &str) -> Result<(), String> {
    let (_ver, sc_arc, _wf_idx) = get_user_scenario_ctx(user_id)?;
    restart_user_iteration_with_scenario(sc_arc.as_ref(), user_id)
}

pub(crate) fn restart_user_iteration_with_scenario(
    sc: &Scenario,
    user_id: &str,
) -> Result<(), String> {
    if let Ok(mut rt) = RUNTIME.lock() {
        let Some(u) = rt.users.get_mut(user_id) else {
            return Ok(());
        };
        for (_nid, t) in u.tasks.iter_mut() {
            // vars/exports 的生命周期仍由 runtime 持有
            t.vars = serde_json::json!({});
            t.exports = serde_json::json!({});
        }
    }
    // states + start enqueue 由 StateMachine 权威重置
    let (_ver, _sc_arc, wf_idx) = get_user_scenario_ctx(user_id)?;
    let effects = {
        let mut sm = crate::STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        sm.apply(
            sc,
            &wf_idx,
            crate::now_ms(),
            SmEvent::UserReset {
                user_id: user_id.to_string(),
            },
        )
    };
    crate::apply_sm_effects(effects)?;
    Ok(())
}
