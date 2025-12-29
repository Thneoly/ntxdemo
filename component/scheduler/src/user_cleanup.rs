//! User finalization / cleanup helpers.
//!
//! Extracted from `lib.rs` so lifecycle + cleanup concerns don't bloat the crate entrypoint.

use std::sync::atomic::Ordering;

use crate::{
    get_bound_udp_owner_id, now_ms, resources, send_scheduler, tx, SchedulerContext, EVENT_COUNTER,
    RUNTIME, STATE_MACHINE, TIMERS,
};

pub(crate) fn finish_user(_ctx: &SchedulerContext, user_id: &str) -> Result<(), String> {
    // 1) read owner id (best-effort)
    let owner_id = {
        let rt = RUNTIME.lock().map_err(|_| "lock runtime".to_string())?;
        rt.users
            .get(user_id)
            .and_then(|u| get_bound_udp_owner_id(&u.resources))
    };

    // 2) remove ready queue entries for this user + drop runtime user
    if let Ok(mut rt) = RUNTIME.lock() {
        rt.ready.retain_user(user_id);
        rt.users.remove(user_id);
    }

    // 2.1) drop state-machine user state (authoritative workflow instance)
    if let Ok(mut sm) = STATE_MACHINE.lock() {
        sm.users.remove(user_id);
        sm.history.remove(user_id);
    }

    // 3) cancel timers for this user
    if let Ok(mut timers) = TIMERS.lock() {
        timers.retain(|t| t.user_id.as_deref() != Some(user_id));
    }

    // 4) cancel send jobs for this user
    send_scheduler::cancel_jobs_for_user(user_id);

    // 5) clear sock ctx mappings
    tx::clear_sock_ctx_for_user(user_id);

    // 6) release host resources (owner) best-effort
    if let Some(owner) = owner_id {
        match resources::release_resource(&owner) {
            Ok(_) => {
                let _ = crate::ntx::scenario_eventbus::event_bus::publish(
                    &crate::ntx::scenario_eventbus::event_bus::Event {
                        id: format!("rr-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)),
                        kind: crate::EventKind::SchedulerResourceReleased
                            .as_str()
                            .to_string(),
                        user_id: Some(user_id.to_string()),
                        task_id: None,
                        action_id: None,
                        payload: serde_json::json!({"owner_id": owner}).to_string(),
                        correlation_id: None,
                        timestamp_ms: now_ms(),
                    },
                );
            }
            Err(e) => {
                println!(
                    "[scheduler] warn: release_resource(owner_id={}) failed: {:?}",
                    owner, e
                );
            }
        }
    }

    Ok(())
}
