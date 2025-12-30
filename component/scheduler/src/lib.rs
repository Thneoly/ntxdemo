//! 调度器组件骨架（wasm32-wasip2）。
//! 仅提供占位实现，便于后续对接状态机与负载控制逻辑。

// This crate is currently an MVP / skeleton. Many items are intentionally not wired yet.
// Keep warnings actionable by silencing dead_code until the integration is completed.
#![allow(dead_code)]
mod bindmod {
    wit_bindgen::generate!({
        world: "ntx:scenario-scheduler/scheduler-main@0.1.0",
        path: [
            "../wit/host",
            "../wit/types",
            "../wit/eventbus",
            "../wit/actions-executor",
            "../wit/scheduler",
        ],
        generate_all,
        generate_unused_types:true,
        debug: true,
    });
}

// -----------------------------------------------------------------------------
// Component exports (WIT)
// -----------------------------------------------------------------------------

/// Root component export type for the `scheduler-main` world.
///
/// The generated bindings expect this crate to export:
/// - `ntx:scenario-scheduler/scheduler-component@0.1.0#run(config-dir: string) -> result<_, string>`
///
/// Without calling `export!(...)`, the linker won't see the required exports and
/// `wasm-component-ld` will fail with "failed to find export ... run".
use crate::bindmod::export;

struct SchedulerExports;

export!(SchedulerExports with_types_in crate::bindmod);

mod eventing;
mod io;
mod net;
mod runtime;
mod scenario;
mod scheduler;

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::io::{rx_pump, tx};
use crate::runtime::runtime_state::{UserMeta, RUNTIME};
use crate::runtime::{lifecycle, status, user_cleanup};
use crate::scenario::scenario_loader::{self, ScenarioConfig};
use crate::scenario::scenario_types::{Action, Scenario, UserLifetime};
use crate::scenario::template;
use crate::scheduler::{dispatch, driver};

use crate::eventing::events::{publish_event, EVENT_COUNTER};
use crate::eventing::payloads::{
    PacketRxEventPayload, ResourceBoundPayload, SchedulerStateChangedPayload,
    SchedulerTimerPayload, SendScheduledPayload, TaskStateChangedPayload, TopologyRejectedPayload,
    UserStartPayload,
};
use crate::eventing::topics::{EventKind, TopicFilter};
use crate::scheduler::state_machine::{SmEffect, StateMachine};

use crate::bindmod::ntx::core_types::types::{
    ActionContext,
    ActionDef,
    ActionOutcome,
    SendRequest,
    SendRequestState,
    SendSchedule,
    // SendStatus,
};
impl bindmod::exports::ntx::scenario_scheduler::scheduler_component::Guest for SchedulerExports {
    fn run(config_dir: String) -> Result<(), String> {
        println!("[scheduler] run with config dir: {config_dir}");

        let scenario = scenario_loader::load_scenario_config(&config_dir)?;
        scenario_loader::log_config_summary(&scenario)?;
        let ctx = SchedulerContext { scenario };

        // IMPORTANT: subscribe first, then publish/init.
        // Our eventbus is best-effort (no durable backlog), so publishing user.start before
        // subscribing to scheduler.user.* would drop the first user.start and stall the workflow.
        let sub_tx = driver::subscribe_or_log(TopicFilter::Exact(EventKind::PacketTxRequest));
        let sub_send = driver::subscribe_or_log(TopicFilter::Exact(EventKind::SendScheduleRequest));
        let sub_ar = driver::subscribe_or_log(TopicFilter::Exact(EventKind::SchedulerActionResult));
        let sub_rx = driver::subscribe_or_log(TopicFilter::Exact(EventKind::PacketRx));
        let sub_ctrl = driver::subscribe_or_log(TopicFilter::SchedulerControlAll);
        let sub_timer = driver::subscribe_or_log(TopicFilter::SchedulerTimerAll);
        let sub_user = driver::subscribe_or_log(TopicFilter::SchedulerUserAll);
        let sub_topo = driver::subscribe_or_log(TopicFilter::Exact(EventKind::TopologyChanged));

        // Start pulling RX from host and publishing `packet.rx`.
        // Must happen after subscribing to `packet.rx` to avoid dropping the first RX events.
        // Note: On runtimes without WASI threads, we do best-effort non-blocking pulls in the
        // event loop (`pump_rx_once_nonblocking`).

        driver::init_runtime(&ctx)?;
        publish_scheduler_state(SchedulerState::Running, None);

        let loop_result = driver::run_event_loop(
            &ctx,
            sub_tx.as_deref(),
            sub_send.as_deref(),
            sub_ar.as_deref(),
            sub_rx.as_deref(),
            sub_ctrl.as_deref(),
            sub_timer.as_deref(),
            sub_user.as_deref(),
            sub_topo.as_deref(),
        );

        publish_scheduler_state(SchedulerState::Completed, None);
        loop_result
    }
}

fn pump_rx_once_nonblocking() {
    rx_pump::pump_rx_once_nonblocking()
}

/// 运行期上下文，占位后续扩展。
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct SchedulerContext {
    scenario: ScenarioConfig,
}

/// Workflow 加速索引（避免每次 packet.rx 扫描全部 wait 节点）
#[derive(Default, Debug, Clone)]
struct WorkflowIndex {
    wait_any: Vec<String>,
    wait_by_action_id: HashMap<String, Vec<String>>,
}

#[derive(serde::Deserialize, Default, Debug, Clone)]
struct PacketRxPayload {
    #[serde(default)]
    sock_id: u64,
    #[serde(default)]
    len: usize,
    #[serde(default)]
    payload_hex: String,
}

use status::SchedulerState;

type TemplateContext = template::TemplateContext;

#[derive(Clone)]
struct SockCtx {
    user_id: Option<String>,
    task_id: Option<String>,
    action_id: Option<String>,
    correlation_id: Option<String>,
    last_seen_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum TaskState {
    Created,
    Ready,
    Running,
    Waiting,
    Completed,
    Failed,
}

static SOCK_CTX: Lazy<Mutex<HashMap<u64, SockCtx>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// StateMachine：权威的 workflow 引擎（per-user task 状态 + 边推进）。
///
/// 约束：TaskRuntime(vars/exports/resources) 仍在 RUNTIME 内；StateMachine 只负责
/// “哪个节点在什么状态、收到什么事件后如何沿 workflow 边推进”。
static STATE_MACHINE: Lazy<Mutex<StateMachine>> = Lazy::new(|| Mutex::new(StateMachine::default()));

fn apply_sm_effects(effects: Vec<SmEffect>) -> Result<(), String> {
    if effects.is_empty() {
        return Ok(());
    }
    let mut rt = RUNTIME.lock().map_err(|_| "lock runtime".to_string())?;
    for e in effects {
        match e {
            SmEffect::SetState {
                user_id,
                node_id,
                state,
            } => {
                if let Some(u) = rt.users.get_mut(&user_id) {
                    if let Some(t) = u.tasks.get_mut(&node_id) {
                        let prev = t.state;
                        t.state = state;
                        if prev != state {
                            let payload = serde_json::to_value(&crate::TaskStateChangedPayload {
                                from: format!("{:?}", prev),
                                to: format!("{:?}", state),
                                scenario_version: u.meta.scenario_version,
                                ts_ms: now_ms(),
                            })
                            .unwrap_or(serde_json::json!({}));
                            publish_event(
                                crate::EventKind::SchedulerTaskStateChanged.as_str(),
                                Some(&user_id),
                                Some(&node_id),
                                None,
                                payload,
                            );
                        }
                    }
                }
            }
            SmEffect::EnqueueReady {
                user_id,
                node_id,
                priority,
            } => {
                rt.ready.push(priority, user_id, node_id);
            }
        }
    }
    Ok(())
}

// ----------------- 运行态初始化与事件循环 -----------------

fn maybe_finish_user(_ctx: &SchedulerContext, user_id: &str) -> Result<(), String> {
    lifecycle::maybe_finish_user(_ctx, user_id)
}

fn publish_user_exit_event(user_id: &str, reason: &str) {
    lifecycle::publish_user_exit_event(user_id, reason)
}

fn restart_user_iteration(_ctx: &SchedulerContext, user_id: &str) -> Result<(), String> {
    lifecycle::restart_user_iteration(_ctx, user_id)
}

fn restart_user_iteration_with_scenario(sc: &Scenario, user_id: &str) -> Result<(), String> {
    lifecycle::restart_user_iteration_with_scenario(sc, user_id)
}

fn user_meta_from_config(ul: &UserLifetime) -> UserMeta {
    let mode = ul.mode;
    let think_ms = ul.think_time.as_deref().and_then(parse_duration_ms);
    let max_running = ul
        .max_concurrency
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(1);
    UserMeta {
        mode,
        iterations: ul.iterations,
        think_ms,
        iteration: 0,
        end_event_sent: false,
        running: 0,
        max_running,
        scenario_version: 1,
    }
}

fn parse_duration_ms(s: &str) -> Option<u64> {
    // minimal parser: "200ms" / "2s" / "1500"
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(v) = t.strip_suffix("ms") {
        return v.trim().parse::<u64>().ok();
    }
    if let Some(v) = t.strip_suffix('s') {
        return v.trim().parse::<u64>().ok().map(|n| n * 1000);
    }
    t.parse::<u64>().ok()
}

/// Build per-user initial `resources` JSON blob from scenario config.
///
/// This is used during `on_user_start_event` to initialize `UserInstance.resources`.
fn build_resources_json(sc: &Scenario) -> serde_json::Value {
    let mut resources = serde_json::json!({});

    // Protocol-specific resource namespaces (UDP today; extensible later).
    crate::net::net_hooks::init_user_resources_for_scenario(sc, &mut resources);

    resources
}

#[allow(dead_code)]
fn cleanup_sock_ctx(now_ms: u64, max_age_ms: u64) {
    if let Ok(mut map) = SOCK_CTX.lock() {
        // Log removals due to TTL expiry.
        let mut removed: u64 = 0;
        map.retain(|sock_id, ctx| {
            let expired = now_ms.saturating_sub(ctx.last_seen_ms) > max_age_ms;
            if expired {
                removed = removed.saturating_add(1);
                println!(
                    "[scheduler][sock_ctx] remove(ttl): sock_id={} age_ms={} user_id={:?} task_id={:?} action_id={:?} corr_id={:?}",
                    sock_id,
                    now_ms.saturating_sub(ctx.last_seen_ms),
                    ctx.user_id,
                    ctx.task_id,
                    ctx.action_id,
                    ctx.correlation_id
                );
            }
            !expired
        });

        if removed > 0 {
            println!(
                "[scheduler][sock_ctx] cleanup(ttl): removed={} remaining={}",
                removed,
                map.len()
            );
        }
    }
}

fn publish_scheduler_state(state: SchedulerState, err: Option<&String>) {
    status::publish_scheduler_state(state, err)
}

fn dispatch_ready_tasks(ctx: &SchedulerContext, max: usize) -> Result<bool, String> {
    dispatch::dispatch_ready_tasks(ctx, max)
}

fn publish_action_result_event(
    user_id: &str,
    task_id: &str,
    action_id: &str,
    correlation_id: Option<&str>,
    outcome: &ActionOutcome,
) -> Result<(), String> {
    dispatch::publish_action_result_event(user_id, task_id, action_id, correlation_id, outcome)
}

fn finish_user(ctx: &SchedulerContext, user_id: &str) -> Result<(), String> {
    user_cleanup::finish_user(ctx, user_id)
}

#[allow(dead_code)]
fn build_action_def_with_ctx(
    action: &Action,
    ctx: &TemplateContext,
    user_id: Option<&str>,
    task_id: Option<&str>,
) -> Result<(ActionDef, ActionContext), String> {
    dispatch::build_action_def_with_ctx(action, ctx, user_id, task_id)
}

/// 在 socket 关闭时清理 sock_id 对应的上下文映射。
pub fn clear_sock_ctx_for_socket(sock_id: u64) {
    tx::clear_sock_ctx_for_socket(sock_id)
}

/// 在 user 结束时清理该 user 相关的所有 sock 上下文。
pub fn clear_sock_ctx_for_user(user_id: &str) {
    tx::clear_sock_ctx_for_user(user_id)
}

fn now_ms() -> u64 {
    crate::scheduler::time::now_ms()
}
