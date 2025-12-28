//! Event handlers for the scheduler event loop.
//!
//! Intentionally thin: these functions orchestrate existing runtime/state-machine helpers
//! defined in `lib.rs`, so we can split files without large-scale redesign.

use crate::{
    ntx, publish_event_with_corr, schedule_timer, validate_scenario, Scenario, SchedulerContext,
    SmEvent, TaskState, WorkflowEdge, EVENT_COUNTER, LOAD, RUNTIME, SCENARIOS, STATE_MACHINE,
};

use std::collections::HashMap;
use std::sync::atomic::Ordering;

// Note: we purposely call back into lib.rs helpers via `crate::...` to avoid moving too much at once.

pub fn on_packet_rx(
    ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    println!("[scheduler] on_packet_rx: invoked");
    handle_packet_rx_trigger(ctx, ev)
}

pub fn on_timer_event(
    ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    match ev.kind.as_str() {
        "scheduler.timer.timeout" => on_timeout_timer(ctx, ev),
        "scheduler.timer.retry" => on_retry_timer(ctx, ev),
        "scheduler.timer.think" => on_think_timer(ctx, ev),
        _ => Ok(()),
    }
}

pub fn on_control_event(ev: &ntx::scenario_eventbus::event_bus::Event) {
    match ev.kind.as_str() {
        "scheduler.control.stop" => {
            if let Ok(mut rt) = RUNTIME.lock() {
                rt.stop = true;
            }
        }
        "scheduler.control.pause" => {
            if let Ok(mut rt) = RUNTIME.lock() {
                rt.paused = true;
            }
        }
        "scheduler.control.resume" => {
            if let Ok(mut rt) = RUNTIME.lock() {
                rt.paused = false;
            }
        }
        _ => {}
    }
}

pub fn on_think_timer(
    ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(&ev.payload).unwrap_or(serde_json::json!({}));
    let user_id = v.get("user_id").and_then(|x| x.as_str()).unwrap_or("");
    if user_id.is_empty() {
        return Ok(());
    }
    crate::restart_user_iteration(ctx, user_id)
}

pub fn on_timeout_timer(
    _ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    // payload: { "user_id": "...", "task_id": "...", "action_id": "..."}
    let v: serde_json::Value = serde_json::from_str(&ev.payload).unwrap_or(serde_json::json!({}));
    let user_id = v.get("user_id").and_then(|x| x.as_str()).unwrap_or("");
    let task_id = v.get("task_id").and_then(|x| x.as_str()).unwrap_or("");
    let action_id = v.get("action_id").and_then(|x| x.as_str()).unwrap_or("");
    if user_id.is_empty() || task_id.is_empty() {
        return Ok(());
    }

    let should_timeout = {
        // ignore if user already gone
        if RUNTIME
            .lock()
            .ok()
            .and_then(|rt| rt.users.get(user_id).map(|_| ()))
            .is_none()
        {
            return Ok(());
        }
        let (_ver, sc_arc, wf_idx) = crate::get_user_scenario_ctx(user_id)?;
        let sc = sc_arc.as_ref();
        let mut sm = STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        let effects = sm.apply(
            sc,
            &wf_idx,
            crate::now_ms(),
            SmEvent::TimeoutTimer {
                user_id: user_id.to_string(),
                node_id: task_id.to_string(),
            },
        );
        let did = !effects.is_empty();
        if did {
            crate::apply_sm_effects(effects)?;
        }
        did
    };

    if should_timeout {
        if let Ok(mut rt) = RUNTIME.lock() {
            if let Some(u) = rt.users.get_mut(user_id) {
                if let Some(t) = u.tasks.get_mut(task_id) {
                    if t.state == TaskState::Running {
                        u.meta.running = u.meta.running.saturating_sub(1);
                    }
                    t.state = TaskState::Failed;
                }
            }
        }

        // publish an action-result(timeout)
        let payload = serde_json::json!({
            "status": "Timeout",
            "detail": "timeout fired",
        })
        .to_string();
        let id = format!("ar-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed));
        let _ =
            ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
                id,
                kind: "scheduler.action-result".to_string(),
                user_id: Some(user_id.to_string()),
                task_id: Some(task_id.to_string()),
                action_id: if action_id.is_empty() {
                    None
                } else {
                    Some(action_id.to_string())
                },
                payload,
                correlation_id: None,
                timestamp_ms: crate::now_ms(),
            });
    }

    Ok(())
}

pub fn on_retry_timer(
    _ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(&ev.payload).unwrap_or(serde_json::json!({}));
    let user_id = v.get("user_id").and_then(|x| x.as_str()).unwrap_or("");
    let task_id = v.get("task_id").and_then(|x| x.as_str()).unwrap_or("");
    if user_id.is_empty() || task_id.is_empty() {
        return Ok(());
    }

    let (_ver, sc_arc, wf_idx) = crate::get_user_scenario_ctx(user_id)?;
    let sc = sc_arc.as_ref();
    let node = sc.workflows.nodes.iter().find(|n| n.id == task_id);
    if node.is_none() {
        return Ok(());
    }
    let effects = {
        let mut sm = STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        sm.apply(
            sc,
            &wf_idx,
            crate::now_ms(),
            SmEvent::RetryTimer {
                user_id: user_id.to_string(),
                node_id: task_id.to_string(),
            },
        )
    };
    crate::apply_sm_effects(effects)?;
    Ok(())
}

pub fn on_topology_changed_event(
    _ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    // This function is large but relatively self-contained; moving it reduces lib.rs size.

    #[derive(Debug, Clone, serde::Deserialize)]
    #[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
    enum PatchOp {
        SetNodePriority {
            node_id: String,
            priority: i32,
        },
        UpsertEdge {
            from: String,
            to: String,
            #[serde(default)]
            label: Option<String>,
            #[serde(default)]
            trigger: Option<serde_json::Value>,
        },
        RemoveNode {
            node_id: String,
        },
        AddNode {
            node: crate::WorkflowNodeDef,
        },
        UpsertAction {
            action: crate::Action,
        },
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TopologyChangedEnvelope {
        schema_version: u32,
        change_id: String,
        #[serde(default)]
        base_version: Option<u64>,
        #[serde(flatten)]
        change: TopologyChangedBody,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    #[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
    enum TopologyChangedBody {
        ReplaceYaml { scenario_yaml: String },
        ReplaceJson { scenario_json: serde_json::Value },
        Patch { ops: Vec<PatchOp> },
    }

    let corr = ev
        .correlation_id
        .as_deref()
        .or_else(|| Some(ev.id.as_str()));

    let env: TopologyChangedEnvelope = match serde_json::from_str(&ev.payload) {
        Ok(v) => v,
        Err(e) => {
            publish_event_with_corr(
                "scheduler.topology.rejected",
                None,
                None,
                None,
                corr,
                serde_json::json!({
                    "error": format!("invalid payload json: {e}"),
                }),
            );
            return Err("invalid topology.changed payload".to_string());
        }
    };

    if env.schema_version != 1 {
        publish_event_with_corr(
            "scheduler.topology.rejected",
            None,
            None,
            None,
            corr,
            serde_json::json!({
                "change_id": env.change_id,
                "error": format!("unsupported schema_version={}", env.schema_version),
            }),
        );
        return Err("unsupported topology schema_version".to_string());
    }

    fn apply_patch(sc: &mut Scenario, ops: &[PatchOp]) -> Result<(), String> {
        for op in ops {
            match op {
                PatchOp::SetNodePriority { node_id, priority } => {
                    let n = sc
                        .workflows
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == *node_id)
                        .ok_or_else(|| format!("set-node-priority: node not found: {}", node_id))?;
                    n.priority = Some(*priority);
                }
                PatchOp::UpsertEdge {
                    from,
                    to,
                    label,
                    trigger,
                } => {
                    let n = sc
                        .workflows
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == *from)
                        .ok_or_else(|| format!("upsert-edge: from node not found: {}", from))?;
                    if let Some(e) = n.edges.iter_mut().find(|e| e.to == *to) {
                        if label.is_some() {
                            e.label = label.clone();
                        }
                        if trigger.is_some() {
                            e.trigger = trigger.clone();
                        }
                    } else {
                        n.edges.push(WorkflowEdge {
                            to: to.clone(),
                            label: label.clone(),
                            trigger: trigger.clone(),
                        });
                    }
                }
                PatchOp::RemoveNode { node_id } => {
                    sc.workflows.nodes.retain(|n| n.id != *node_id);
                    for n in sc.workflows.nodes.iter_mut() {
                        n.edges.retain(|e| e.to != *node_id);
                    }
                }
                PatchOp::AddNode { node } => {
                    if sc.workflows.nodes.iter().any(|n| n.id == node.id) {
                        return Err(format!("add-node: duplicate node id: {}", node.id));
                    }
                    sc.workflows.nodes.push(node.clone());
                }
                PatchOp::UpsertAction { action } => {
                    if let Some(a) = sc.actions.actions.iter_mut().find(|a| a.id == action.id) {
                        *a = action.clone();
                    } else {
                        sc.actions.actions.push(action.clone());
                    }
                }
            }
        }
        Ok(())
    }

    let (base_ver, base_sc) = {
        let reg = SCENARIOS
            .lock()
            .map_err(|_| "lock scenario registry".to_string())?;
        let (active_ver, sc, _idx) = reg
            .active()
            .ok_or_else(|| "no active scenario".to_string())?;
        let want = env.base_version.unwrap_or(active_ver);
        if want != active_ver {
            publish_event_with_corr(
                "scheduler.topology.rejected",
                None,
                None,
                None,
                corr,
                serde_json::json!({
                    "change_id": env.change_id,
                    "error": format!("base_version mismatch: want={}, active={}", want, active_ver),
                    "base_version": want,
                    "active_version": active_ver,
                }),
            );
            return Err("base_version mismatch".to_string());
        }
        (active_ver, sc)
    };

    let mode_str = match &env.change {
        TopologyChangedBody::ReplaceYaml { .. } => "replace-yaml",
        TopologyChangedBody::ReplaceJson { .. } => "replace-json",
        TopologyChangedBody::Patch { .. } => "patch",
    };

    let new_sc: Scenario = match env.change {
        TopologyChangedBody::ReplaceYaml { scenario_yaml } => {
            serde_yaml::from_str::<Scenario>(&scenario_yaml)
                .or_else(|_| serde_json::from_str::<Scenario>(&scenario_yaml))
                .map_err(|e| format!("parse replace-yaml: {e}"))?
        }
        TopologyChangedBody::ReplaceJson { scenario_json } => {
            serde_json::from_value::<Scenario>(scenario_json)
                .map_err(|e| format!("parse replace-json: {e}"))?
        }
        TopologyChangedBody::Patch { ops } => {
            let mut sc = (*base_sc).clone();
            apply_patch(&mut sc, &ops)?;
            sc
        }
    };

    if let Err(e) = validate_scenario(&new_sc) {
        publish_event_with_corr(
            "scheduler.topology.rejected",
            None,
            None,
            None,
            corr,
            serde_json::json!({
                "change_id": env.change_id,
                "base_version": base_ver,
                "mode": mode_str,
                "error": e,
            }),
        );
        return Err("topology validation failed".to_string());
    }

    let new_ver = {
        let mut reg = SCENARIOS
            .lock()
            .map_err(|_| "lock scenario registry".to_string())?;
        reg.install_new_active(new_sc.clone())
    };

    if let Ok(mut lc) = LOAD.lock() {
        if let Ok(reg) = SCENARIOS.lock() {
            if let Some((sc, _idx)) = reg.by_version(new_ver) {
                lc.phases = sc.load.ramp_up.phases.clone();
                lc.next_phase = 0;
            }
        }
    }

    publish_event_with_corr(
        "scheduler.topology.applied",
        None,
        None,
        None,
        corr,
        serde_json::json!({
            "change_id": env.change_id,
            "base_version": base_ver,
            "new_version": new_ver,
            "mode": mode_str,
        }),
    );
    println!(
        "[scheduler] topology.changed applied: new active_version={}",
        new_ver
    );

    Ok(())
}

pub fn on_user_start_event(
    _ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let (ver, sc_arc, wf_idx) = crate::get_active_scenario_ctx()?;
    let sc = sc_arc.as_ref();
    let v: serde_json::Value = serde_json::from_str(&ev.payload).unwrap_or(serde_json::json!({}));
    let user_id = v.get("user_id").and_then(|x| x.as_str()).unwrap_or("");
    if user_id.is_empty() {
        return Ok(());
    }

    eprintln!(
        "[scheduler] on_user_start: user_id={user_id} scenario_version={ver} workflows_nodes={} start_nodes={:?}",
        sc.workflows.nodes.len(),
        crate::find_start_nodes(sc)
    );

    let resources = crate::build_resources_json(sc);
    let mut user = crate::UserInstance {
        tasks: HashMap::new(),
        resources,
        meta: {
            let mut m = crate::user_meta_from_config(&sc.load.user_lifetime);
            m.scenario_version = ver;
            m
        },
    };

    for n in &sc.workflows.nodes {
        let tr = crate::TaskRuntime {
            state: TaskState::Created,
            vars: serde_json::json!({}),
            exports: serde_json::json!({}),
        };
        user.tasks.insert(n.id.clone(), tr);
    }

    if let Ok(mut rt) = RUNTIME.lock() {
        if rt.users.contains_key(user_id) {
            eprintln!(
                "[scheduler] on_user_start: user already exists, ignoring: user_id={user_id}"
            );
            return Ok(());
        }
        rt.users.insert(user_id.to_string(), user);
    }

    let effects = {
        let mut sm = STATE_MACHINE
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

    eprintln!(
        "[scheduler] on_user_start: sm.apply(UserReset) effects_len={} user_id={user_id}",
        effects.len()
    );
    crate::apply_sm_effects(effects)?;

    if let Ok(rt) = RUNTIME.lock() {
        let running = rt
            .users
            .get(user_id)
            .map(|u| (u.meta.running, u.meta.max_running))
            .unwrap_or((0, 0));
        eprintln!(
            "[scheduler] on_user_start: after effects user_id={user_id} ready_empty={} running={}/{}",
            rt.ready.is_empty(),
            running.0,
            running.1
        );
    }

    Ok(())
}

pub fn on_user_exit_event(
    _ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let Some(user_id) = ev.user_id.as_deref() else {
        return Ok(());
    };
    let (_ver, sc_arc, _wf_idx) = crate::get_user_scenario_ctx(user_id)?;
    let sc = sc_arc.as_ref();

    let (mode, iterations, think_ms, cur_iter) = {
        let rt = RUNTIME.lock().map_err(|_| "lock runtime".to_string())?;
        let Some(u) = rt.users.get(user_id) else {
            return Ok(());
        };
        (
            u.meta.mode.clone(),
            u.meta.iterations,
            u.meta.think_ms,
            u.meta.iteration,
        )
    };

    if mode != "loop" {
        return crate::finish_user(_ctx, user_id);
    }

    let next_iter = cur_iter.saturating_add(1);
    let should_stop = iterations.map(|n| next_iter >= n).unwrap_or(false);

    if let Ok(mut rt) = RUNTIME.lock() {
        if let Some(u) = rt.users.get_mut(user_id) {
            u.meta.iteration = next_iter;
            u.meta.end_event_sent = false;
        }
    }

    if should_stop {
        return crate::finish_user(_ctx, user_id);
    }

    if let Some(ms) = think_ms {
        schedule_timer(
            "scheduler.timer.think",
            crate::now_ms().saturating_add(ms),
            user_id,
            "user",
            None,
            serde_json::json!({"user_id": user_id, "iteration": next_iter}),
        );
        Ok(())
    } else {
        crate::restart_user_iteration_with_scenario(sc, user_id)
    }
}

fn handle_packet_rx_trigger(
    ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let Some(ctx_user) = ev.user_id.as_deref() else {
        println!("[scheduler] handle_packet_rx_trigger: missing user_id");
        return Ok(());
    };
    let action_id = ev.action_id.as_deref().unwrap_or("");
    let task_id = ev.task_id.as_deref().unwrap_or("");
    let (_ver, sc_arc, wf_idx) = crate::get_user_scenario_ctx(ctx_user)?;
    let sc = sc_arc.as_ref();

    let p: crate::PacketRxPayload = serde_json::from_str(&ev.payload).unwrap_or_default();
    let eval_ctx = serde_json::json!({
        "event": "packet.rx",
        "reason": "packet.rx",
        "user_id": ctx_user,
        "task_id": task_id,
        "action_id": action_id,
        "sock_id": p.sock_id,
        "len": p.len,
        "payload_hex": p.payload_hex,
    });

    let effects = {
        let mut sm = STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        sm.apply(
            sc,
            &wf_idx,
            crate::now_ms(),
            SmEvent::PacketRx {
                user_id: ctx_user.to_string(),
                action_id: action_id.to_string(),
                task_id: task_id.to_string(),
                payload: p,
                eval_ctx,
            },
        )
    };

    crate::apply_sm_effects(effects)?;
    crate::maybe_finish_user(ctx, ctx_user)?;
    Ok(())
}
