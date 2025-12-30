//! Protocol-specific hooks.
//!
//! Goal: keep protocol quirks (UDP today; TCP etc. later) out of core scheduler logic.
//!
//! Add new protocol behaviors by extending these hooks instead of scattering
//! `if action.call.is_...()` across the codebase.

use crate::bindmod::ntx::core_types::types::ActionDef;
use crate::bindmod::ntx::host::udp_socket_control;
use crate::net::{net_actions::ActionCall, udp_binding};
use crate::scenario::scenario_types::{Action, Scenario};
use crate::SchedulerContext;

pub(crate) fn get_bound_owner_id(resources: &serde_json::Value) -> Option<String> {
    // Today: UDP uses a single owner id for all bound host resources.
    // Future protocols can extend this to consult their own namespaces.
    udp_binding::get_bound_udp_owner_id(resources)
}

/// Initialize per-user resources JSON for a given scenario.
///
/// This is invoked when creating a new `UserInstance`.
pub(crate) fn init_user_resources_for_scenario(sc: &Scenario, resources: &mut serde_json::Value) {
    // Today we only model UDP as a bindable resource.
    // Future protocols should add their own resource namespaces here.
    let needs_udp = sc
        .actions
        .actions
        .iter()
        .any(|a| matches!(a.call, ActionCall::UdpSendReply));

    if needs_udp {
        resources
            .as_object_mut()
            .expect("resources json object")
            .insert(
                "udp".to_string(),
                serde_json::json!({
                    "sock_id": serde_json::Value::Null,
                    "owner_id": serde_json::Value::Null,
                }),
            );
    }
}

/// Hook invoked before dispatching an action.
///
/// Use this for protocol-level preparation (e.g. bind socket for a user).
pub(crate) fn before_dispatch_action(
    ctx: &SchedulerContext,
    sc: &Scenario,
    user_id: &str,
    action: &Action,
) -> Result<(), String> {
    match action.call {
        ActionCall::UdpSendReply => udp_binding::ensure_udp_socket_for_user(ctx, sc, user_id),
        ActionCall::Other => Ok(()),
    }
}

/// Hook invoked after building an `ActionDef` but before executing it.
///
/// Use this to inject protocol-specific parameters into the `ActionDef`.
pub(crate) fn mutate_action_def_before_execute(
    user_id: &str,
    action: &Action,
    def: &mut ActionDef,
) -> Result<(), String> {
    match action.call {
        ActionCall::UdpSendReply => udp_binding::inject_udp_socket_id(user_id, def),
        ActionCall::Other => Ok(()),
    }
}

/// Protocol-aware send primitive used by the scheduler.
///
/// Today it represents UDP socket send-reply semantics.
/// Future protocols (e.g. TCP) should be added here so callers don't need to change.
pub(crate) fn send_on_socket(
    sock_id: u64,
    payload: &[u8],
    user_id: Option<&str>,
    task_id: Option<&str>,
    action_id: Option<&str>,
    correlation_id: Option<&str>,
) -> Result<(), String> {
    let now_ms = crate::scheduler::time::now_ms();

    // Record sock context for later packet.rx correlation.
    if let Ok(mut map) = crate::SOCK_CTX.lock() {
        let new_ctx = crate::SockCtx {
            user_id: user_id.map(|s| s.to_string()),
            task_id: task_id.map(|s| s.to_string()),
            action_id: action_id.map(|s| s.to_string()),
            correlation_id: correlation_id.map(|s| s.to_string()),
            last_seen_ms: now_ms,
        };

        match map.insert(sock_id, new_ctx.clone()) {
            None => {
                println!(
                    "[scheduler][sock_ctx] add: sock_id={} user_id={:?} task_id={:?} action_id={:?} corr_id={:?}",
                    sock_id,
                    new_ctx.user_id,
                    new_ctx.task_id,
                    new_ctx.action_id,
                    new_ctx.correlation_id
                );
            }
            Some(prev) => {
                // Overwrite is expected when the same socket is reused across actions.
                println!(
                    "[scheduler][sock_ctx] update: sock_id={} prev(user_id={:?} task_id={:?} action_id={:?} corr_id={:?}) -> new(user_id={:?} task_id={:?} action_id={:?} corr_id={:?})",
                    sock_id,
                    prev.user_id,
                    prev.task_id,
                    prev.action_id,
                    prev.correlation_id,
                    new_ctx.user_id,
                    new_ctx.task_id,
                    new_ctx.action_id,
                    new_ctx.correlation_id
                );
            }
        }
    }

    let frame = udp_socket_control::build_reply(sock_id, payload)
        .map_err(|e| format!("build_reply failed: {:?}", e))?;
    udp_socket_control::tx(frame).map_err(|e| format!("tx failed: {:?}", e))?;

    Ok(())
}
