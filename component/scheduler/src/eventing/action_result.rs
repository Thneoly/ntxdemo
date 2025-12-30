//! Handling of `scheduler.action-result` events.
//!
//! This is intentionally kept as a single module because it touches runtime state,
//! state-machine step tracking, retry timers and workflow advancement.

use crate::eventing::events::EVENT_COUNTER;
use crate::eventing::payloads::{EvalCtx, EvalReason};
use crate::eventing::topics::EventKind;
use crate::runtime::runtime_state::RUNTIME;
use crate::scenario::scenario_types::Action;
use crate::scheduler::state_machine::SmEvent;
use crate::scheduler::timers::schedule_timer;
use crate::{bindmod::ntx, SchedulerContext, TaskState, STATE_MACHINE};

use std::sync::atomic::Ordering;

pub fn on_action_result_event(
    ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    // payload: {status, detail, exports}
    let v: serde_json::Value = serde_json::from_str(&ev.payload).unwrap_or(serde_json::json!({}));
    let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("");
    let detail = v.get("detail").cloned().unwrap_or(serde_json::Value::Null);
    let exports = v.get("exports").cloned();

    let Some(user_id) = ev.user_id.as_deref() else {
        return Ok(());
    };
    let Some(task_id) = ev.task_id.as_deref() else {
        return Ok(());
    };

    let (_ver, sc_arc, wf_idx) =
        crate::scenario::scenario_registry::get_user_scenario_ctx(user_id)?;
    let sc = sc_arc.as_ref();

    let status_lc = status.to_ascii_lowercase();
    let success = status_lc.contains("success");
    let timeout = status_lc.contains("timeout");
    let reason = if success {
        EvalReason::Success
    } else if timeout {
        EvalReason::Timeout
    } else {
        EvalReason::Failed
    };
    let eval_ctx = serde_json::to_value(EvalCtx::action_result(
        user_id,
        task_id,
        ev.action_id.as_deref().unwrap_or(""),
        reason,
        status,
        detail.clone(),
        exports.clone().unwrap_or(serde_json::Value::Null),
    ))
    .unwrap_or(serde_json::json!({}));

    // update runtime state + exports + step branching decision
    let mut need_retry = false;
    let mut retry_after_ms: Option<u64> = None;
    let mut retries_left: Option<i64> = None;

    // step branching (state-machine internal step_idx drives the actual dispatch)
    let cur_step: u32 = {
        let sm = STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        sm.get_step(user_id, task_id)
    };

    let mut jump_step: Option<u32> = None; // Some(next_step) means re-enqueue same node to that step
    {
        if let Ok(mut rt) = RUNTIME.lock() {
            if let Some(u) = rt.users.get_mut(user_id) {
                if let Some(t) = u.tasks.get_mut(task_id) {
                    let was_running = t.state == TaskState::Running;
                    if was_running {
                        u.meta.running = u.meta.running.saturating_sub(1);
                    }
                    if let Some(exp) = exports {
                        if let serde_json::Value::String(s) = exp {
                            if let Ok(j) = serde_json::from_str::<serde_json::Value>(&s) {
                                t.exports = j;
                            }
                        } else if exp.is_object() || exp.is_array() {
                            t.exports = exp;
                        }
                    }

                    // retry policy from vars._retry { left, backoff_ms }
                    if !success {
                        let left = t
                            .vars
                            .get("_retry")
                            .and_then(|r| r.get("left"))
                            .and_then(|n| n.as_i64())
                            .unwrap_or(0);
                        let backoff = t
                            .vars
                            .get("_retry")
                            .and_then(|r| r.get("backoff_ms"))
                            .and_then(|n| n.as_u64())
                            .unwrap_or(1000);
                        retries_left = Some(left);
                        if left > 0 {
                            // decrement
                            if let Some(obj) =
                                t.vars.get_mut("_retry").and_then(|x| x.as_object_mut())
                            {
                                obj.insert(
                                    "left".to_string(),
                                    serde_json::Value::Number((left - 1).into()),
                                );
                            }
                            need_retry = true;
                            retry_after_ms = Some(backoff);
                        }
                    }
                }
            }
        }
    }

    // decide step jump:
    // - success: go to next step if exists
    // - failure/timeout: if no retry and step specifies on_failed_step/on_timeout_step, jump there
    if let Some(node) = sc.workflows.nodes.iter().find(|n| n.id == task_id) {
        let steps_len: usize = if let Some(steps) = node.steps.as_ref().filter(|v| !v.is_empty()) {
            steps.len()
        } else if let Some(actions) = node.actions.as_ref().filter(|v| !v.is_empty()) {
            actions.len()
        } else if node.action.is_some() {
            1
        } else {
            0
        };

        if steps_len > 0 {
            if success {
                let next = cur_step.saturating_add(1);
                if (next as usize) < steps_len {
                    jump_step = Some(next);
                }
            } else if !need_retry {
                if let Some(steps) = node.steps.as_ref().filter(|v| !v.is_empty()) {
                    let si = usize::try_from(cur_step).unwrap_or(0);
                    if si < steps.len() {
                        let st = &steps[si];
                        if timeout {
                            jump_step = st.on_timeout_step;
                        } else {
                            jump_step = st.on_failed_step;
                        }
                        if let Some(ns) = jump_step {
                            if (ns as usize) >= steps_len {
                                jump_step = None;
                            }
                        }
                    }
                }
            }
        }
    }

    // if we will jump to another step, clear per-step retry state so dispatch can re-init
    if jump_step.is_some() {
        if let Ok(mut rt) = RUNTIME.lock() {
            if let Some(u) = rt.users.get_mut(user_id) {
                if let Some(t) = u.tasks.get_mut(task_id) {
                    if let Some(obj) = t.vars.as_object_mut() {
                        obj.remove("_retry");
                        obj.remove("_retry_step");
                    }
                }
            }
        }
    }

    // schedule retry timer as event
    if need_retry {
        let after = retry_after_ms.unwrap_or(1000);
        schedule_timer(
            EventKind::SchedulerTimerRetry.as_str(),
            crate::now_ms().saturating_add(after),
            user_id,
            task_id,
            ev.action_id.as_deref(),
            serde_json::to_value(crate::SchedulerTimerPayload {
                user_id: Some(user_id.to_string()),
                task_id: Some(task_id.to_string()),
                action_id: ev.action_id.clone(),
                left: retries_left,
                iteration: None,
            })
            .unwrap_or(serde_json::json!({})),
        );
    }

    // workflow推进：成功/失败/超时都可以有不同分支；但若还有retry，则延后失败/超时分支推进
    let reason = reason.as_str();

    // should advance workflow edges only when:
    // - success and no jump_step
    // - failure/timeout and no retry and no jump_step
    let should_advance =
        (success && jump_step.is_none()) || (!success && !need_retry && jump_step.is_none());

    let effects = {
        let mut sm = STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        if let Some(ns) = jump_step {
            sm.set_step(user_id, task_id, ns, crate::now_ms());
        }
        sm.apply(
            sc,
            &wf_idx,
            crate::now_ms(),
            SmEvent::ActionResult {
                user_id: user_id.to_string(),
                node_id: task_id.to_string(),
                reason: reason.to_string(),
                success,
                should_advance,
                continue_node: jump_step.is_some(),
                eval_ctx,
            },
        )
    };

    crate::apply_sm_effects(effects)?;
    if should_advance {
        crate::maybe_finish_user(ctx, user_id)?;
    }

    Ok(())
}

// Kept for compatibility: this file previously referenced `Action` in the original
// `lib.rs` section via surrounding code. (No direct use currently.)
#[allow(dead_code)]
fn _keep_action_type_used(_a: &Action) {
    let _ = EVENT_COUNTER.load(Ordering::Relaxed);
}
