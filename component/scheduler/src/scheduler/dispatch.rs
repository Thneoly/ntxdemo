//! Action dispatch / execution.
//!
//! Extracted from `lib.rs` to keep the crate entrypoint small.

use crate::eventing::events::EVENT_COUNTER;
use crate::eventing::payloads::ActionResultPayload;
use crate::eventing::topics::EventKind;
use crate::net::udp_binding;
use crate::ntx::core_types::types::{ActionContext, ActionDef, ActionOutcome};
use crate::runtime::runtime_state::RUNTIME;
use crate::scenario::template::TemplateContext;
use crate::scheduler::state_machine::SmEvent;
use crate::{SchedulerContext, TaskState, STATE_MACHINE};

use super::timers::schedule_timer;
use super::workflow_helpers::node_priority;

pub(crate) fn dispatch_ready_tasks(ctx: &SchedulerContext, max: usize) -> Result<bool, String> {
    let mut did = false;

    for _ in 0..max {
        let next = {
            let mut rt = RUNTIME.lock().map_err(|_| "lock runtime".to_string())?;
            rt.ready.pop_next()
        };
        let Some((user_id, node_id)) = next else {
            break;
        };

        eprintln!("[scheduler] dispatch: pop_ready user_id={user_id} node_id={node_id}");

        // per-user scenario (old users do not migrate)
        let (_ver, sc_arc, wf_idx) =
            match crate::scenario::scenario_registry::get_user_scenario_ctx(&user_id) {
                Ok(v) => v,
                Err(_) => continue,
            };
        let sc = sc_arc.as_ref();

        // Find node
        let node = match sc.workflows.nodes.iter().find(|n| n.id == node_id) {
            Some(n) => n,
            None => continue,
        };

        // multi-step: pick action by state-machine step (authoritative)
        let step_idx: u32 = {
            let sm = STATE_MACHINE
                .lock()
                .map_err(|_| "lock state-machine".to_string())?;
            sm.get_step(&user_id, &node_id)
        };

        eprintln!("[scheduler] dispatch: resolve_step user_id={user_id} node_id={node_id} step_idx={step_idx}");

        let (action_id, step_timeout_ms, step_retry) = {
            // prefer steps
            if let Some(steps) = node.steps.as_ref().filter(|v| !v.is_empty()) {
                let si = usize::try_from(step_idx).unwrap_or(0);
                if si >= steps.len() {
                    continue;
                }
                let st = &steps[si];
                (st.action.clone(), st.timeout_ms, st.retry.clone())
            } else {
                // fallback: actions/action list
                let list: Vec<String> = node
                    .actions
                    .as_ref()
                    .filter(|v| !v.is_empty())
                    .cloned()
                    .or_else(|| node.action.as_ref().map(|a| vec![a.clone()]))
                    .unwrap_or_default();
                let si = usize::try_from(step_idx).unwrap_or(0);
                if list.is_empty() || si >= list.len() {
                    continue;
                }
                (list[si].clone(), None, None)
            }
        };

        eprintln!("[scheduler] dispatch: select_action user_id={user_id} node_id={node_id} action_id={action_id}");

        let action = match sc.actions.actions.iter().find(|a| a.id == action_id) {
            Some(a) => a,
            None => continue,
        };

        eprintln!(
            "[scheduler] dispatch: action_call user_id={user_id} node_id={node_id} call={}",
            action.call.as_call_str()
        );

        // Ensure UDP socket binding if needed
        if action.call.is_udp() {
            udp_binding::ensure_udp_socket_for_user(ctx, sc, &user_id)?;
        }

        // per-user concurrency cap + state transition
        let (task_vars, task_exports, user_resources) = {
            // 1) concurrency check
            {
                let mut rt = RUNTIME.lock().map_err(|_| "lock runtime".to_string())?;
                let u = rt
                    .users
                    .get_mut(&user_id)
                    .ok_or_else(|| format!("user not found: {}", user_id))?;
                if u.meta.running >= u.meta.max_running {
                    eprintln!(
                        "[scheduler] dispatch: concurrency_cap user_id={user_id} running={}/{} requeue node_id={node_id}",
                        u.meta.running,
                        u.meta.max_running
                    );
                    rt.ready.push(
                        node_priority(sc, &node_id),
                        user_id.to_string(),
                        node_id.to_string(),
                    );
                    continue;
                }
            }

            // 2) state-machine transition
            let effects = {
                let mut sm = STATE_MACHINE
                    .lock()
                    .map_err(|_| "lock state-machine".to_string())?;
                sm.apply(
                    sc,
                    &wf_idx,
                    crate::now_ms(),
                    SmEvent::DispatchStart {
                        user_id: user_id.to_string(),
                        node_id: node_id.to_string(),
                    },
                )
            };
            if effects.is_empty() {
                eprintln!("[scheduler] dispatch: stale_ready user_id={user_id} node_id={node_id}");
                continue;
            }
            crate::apply_sm_effects(effects)?;

            // 3) update runtime + snapshot
            let mut rt = RUNTIME.lock().map_err(|_| "lock runtime".to_string())?;
            let u = rt
                .users
                .get_mut(&user_id)
                .ok_or_else(|| format!("user not found: {}", user_id))?;
            let t = u
                .tasks
                .get_mut(&node_id)
                .ok_or_else(|| format!("task not found: {}", node_id))?;
            t.state = TaskState::Running;
            u.meta.running = u.meta.running.saturating_add(1);
            (t.vars.clone(), t.exports.clone(), u.resources.clone())
        };

        let tctx = TemplateContext {
            vars: task_vars,
            resources: user_resources,
            exports: task_exports,
        };

        // Initialize retry policy (kept in lib.rs runtime vars)
        if let Ok(mut rt) = RUNTIME.lock() {
            if let Some(u) = rt.users.get_mut(&user_id) {
                if let Some(t) = u.tasks.get_mut(&node_id) {
                    let cur_step = step_idx;
                    let retry_step = t
                        .vars
                        .get("_retry_step")
                        .and_then(|v| v.as_u64())
                        .and_then(|n| u32::try_from(n).ok());
                    let need_reset = retry_step != Some(cur_step);
                    if need_reset {
                        if let Some(obj) = t.vars.as_object_mut() {
                            obj.remove("_retry");
                            obj.insert(
                                "_retry_step".to_string(),
                                serde_json::Value::Number((cur_step as u64).into()),
                            );
                        }
                    }
                    if t.vars.get("_retry").is_none() {
                        let (max, backoff_ms) = if let Some(r) = step_retry.as_ref() {
                            (r.max, r.backoff_ms)
                        } else {
                            (
                                action
                                    .with
                                    .get("retry")
                                    .and_then(|r| r.get("max"))
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0),
                                action
                                    .with
                                    .get("retry")
                                    .and_then(|r| r.get("backoff_ms"))
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(1000),
                            )
                        };
                        if max > 0 {
                            if let Some(obj) = t.vars.as_object_mut() {
                                obj.insert(
                                    "_retry".to_string(),
                                    serde_json::json!({"left": max, "backoff_ms": backoff_ms}),
                                );
                            }
                        }
                    }
                }
            }
        }

        let (mut def, act_ctx) =
            build_action_def_with_ctx(action, &tctx, Some(&user_id), Some(&node_id))?;
        if action.call.is_udp() {
            udp_binding::inject_udp_socket_id(&user_id, &mut def)?;
        }

        // Timeout timer
        let timeout_ms = step_timeout_ms.or_else(|| {
            action
                .with
                .get("timeout_ms")
                .or_else(|| action.with.get("timeout-ms"))
                .and_then(|v| v.as_u64())
        });
        if let Some(tmo) = timeout_ms {
            schedule_timer(
                EventKind::SchedulerTimerTimeout.as_str(),
                crate::now_ms().saturating_add(tmo),
                &user_id,
                &node_id,
                Some(&def.id),
                serde_json::json!({"user_id": user_id, "task_id": node_id, "action_id": def.id}),
            );
        }

        eprintln!(
            "[scheduler] dispatch: execute_action user_id={user_id} node_id={node_id} action_id={} corr_id={}",
            def.id,
            act_ctx.correlation_id.as_deref().unwrap_or("<none>")
        );

        let outcome = crate::ntx::scenario_actions_executor::action_component::execute_action(
            &def,
            Some(&act_ctx),
        )
        .map_err(|e| format!("execute_action failed: {e}"))?;

        eprintln!(
            "[scheduler] dispatch: action_outcome user_id={user_id} node_id={node_id} action_id={} status={:?}",
            def.id, outcome.status
        );

        publish_action_result_event(
            &user_id,
            &node_id,
            &def.id,
            act_ctx.correlation_id.as_deref(),
            &outcome,
        )?;

        eprintln!(
            "[scheduler] dispatch: published action-result user_id={user_id} node_id={node_id} action_id={}",
            def.id
        );

        did = true;
    }

    Ok(did)
}

pub(crate) fn publish_action_result_event(
    user_id: &str,
    task_id: &str,
    action_id: &str,
    correlation_id: Option<&str>,
    outcome: &ActionOutcome,
) -> Result<(), String> {
    let now = crate::now_ms();
    let metrics = outcome.metrics.as_ref().map(|m| {
        serde_json::json!({
            "latency_ms": m.latency_ms,
            "bytes_sent": m.bytes_sent,
            "bytes_received": m.bytes_received,
            "response_code": m.response_code,
        })
    });
    let detail_json: serde_json::Value = match &outcome.detail {
        Some(s) => serde_json::Value::String(s.clone()),
        None => serde_json::Value::Null,
    };
    let exports_json: Option<serde_json::Value> = outcome
        .exports
        .as_ref()
        .map(|s| serde_json::Value::String(s.clone()));

    let payload = serde_json::to_string(&ActionResultPayload {
        status: format!("{:?}", outcome.status),
        detail: detail_json,
        metrics,
        exports: exports_json,
    })
    .unwrap_or_else(|_| "{}".to_string());

    let id = format!(
        "ar-{}",
        EVENT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    crate::ntx::scenario_eventbus::event_bus::publish(
        &crate::ntx::scenario_eventbus::event_bus::Event {
            id,
            kind: EventKind::SchedulerActionResult.as_str().to_string(),
            user_id: Some(user_id.to_string()),
            task_id: Some(task_id.to_string()),
            action_id: Some(action_id.to_string()),
            payload,
            correlation_id: correlation_id.map(|s| s.to_string()),
            timestamp_ms: now,
        },
    )
    .map_err(|e| format!("publish scheduler.action-result: {e}"))?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn build_action_def_with_ctx(
    action: &crate::Action,
    ctx: &TemplateContext,
    user_id: Option<&str>,
    task_id: Option<&str>,
) -> Result<(ActionDef, ActionContext), String> {
    let expanded = crate::scenario::template::render_value(&action.with, ctx)?;
    let params = serde_json::to_string(&expanded).map_err(|e| format!("encode params: {e}"))?;

    let def = ActionDef {
        id: action.id.clone(),
        call: action.call.as_call_str().to_string(),
        params,
        exports: vec![],
    };

    let act_ctx = ActionContext {
        user_id: user_id.map(|s| s.to_string()),
        task_id: task_id.map(|s| s.to_string()),
        action_id: Some(action.id.clone()),
        correlation_id: Some(format!(
            "corr-{}",
            EVENT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )),
        vars: Some(ctx.vars.to_string()),
        resources: Some(ctx.resources.to_string()),
        deadline_ms: None,
    };

    Ok((def, act_ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_with_is_encoded_as_json_params_string() {
        let action = crate::scenario::scenario_types::Action {
            id: "a1".to_string(),
            call: crate::scenario::scenario_types::ActionCall::UdpSendReply,
            with: serde_json::json!({
                "payload_hex": "010203",
                "timeout_ms": 1500,
                "retry": {"max": 2, "backoff_ms": 10}
            }),
        };

        let ctx = TemplateContext {
            vars: serde_json::json!({}),
            resources: serde_json::json!({}),
            exports: serde_json::json!({}),
        };

        let (def, _act_ctx) = build_action_def_with_ctx(&action, &ctx, Some("u1"), Some("t1"))
            .expect("build action def");

        // Contract: params is a JSON string (with_json), not a structured map.
        let v: serde_json::Value =
            serde_json::from_str(&def.params).expect("params should be valid JSON");
        assert_eq!(v["payload_hex"], "010203");
        assert_eq!(v["timeout_ms"], 1500);
        assert_eq!(v["retry"]["max"], 2);
        assert_eq!(v["retry"]["backoff_ms"], 10);
    }
}
