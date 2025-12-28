//! 调度器组件骨架（wasm32-wasip2）。
//! 仅提供占位实现，便于后续对接状态机与负载控制逻辑。

// This crate is currently an MVP / skeleton. Many items are intentionally not wired yet.
// Keep warnings actionable by silencing dead_code until the integration is completed.
#![allow(dead_code)]

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

mod action_result;
mod codec;
mod conditions;
mod events;
mod handlers;
mod load_controller;
mod runtime_state;
mod rx_decode;
mod scenario_registry;
mod send_scheduler;
mod state_machine;
mod template;
mod time;
mod timers;
mod tx;
mod udp_binding;
mod workflow_helpers;

pub(crate) use workflow_helpers::{
    edge_trigger_allows, find_start_nodes, node_priority, wait_match,
};

pub(crate) use events::publish_event_with_corr;
use events::{publish_event, EVENT_COUNTER};

pub(crate) use conditions::{eval_condition, match_reason};

pub(crate) use runtime_state::{TaskRuntime, UserInstance, UserMeta, RUNTIME};
pub(crate) use scenario_registry::{get_active_scenario_ctx, get_user_scenario_ctx, SCENARIOS};

pub(crate) use load_controller::{publish_user_start_event, tick_load_controller, LOAD};
use state_machine::{SmEffect, SmEvent, StateMachine};
pub(crate) use timers::{schedule_timer, tick_timers, TIMERS};

use crate::ntx::core_types::types::{
    ActionContext,
    ActionDef,
    ActionOutcome,
    SendRequest,
    SendRequestState,
    SendSchedule,
    // SendStatus,
};
use crate::ntx::host::{resources, types, udp_socket_control};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
struct SchedulerExports;

// Emit the component exports for the `scheduler-main` world.
// Without this, the WIT interface functions (e.g. scheduler-component.run)
// won't be visible to the component encoder/linker.
export!(SchedulerExports);

#[derive(Clone, Debug)]
struct ScenarioConfig {
    config_dir: String,
    workflow_raw: Option<String>,
    workbook_raw: Option<String>,
    actions_raw: Option<String>,
    load_raw: Option<String>,
    parsed: Option<Scenario>,
}

impl exports::ntx::scenario_scheduler::scheduler_component::Guest for SchedulerExports {
    fn run(config_dir: String) -> Result<(), String> {
        println!("[scheduler] run with config dir: {config_dir}");

        let scenario = load_scenario_config(&config_dir)?;
        log_config_summary(&scenario)?;
        let ctx = SchedulerContext { scenario };

        // IMPORTANT: subscribe first, then publish/init.
        // Our eventbus is best-effort (no durable backlog), so publishing user.start before
        // subscribing to scheduler.user.* would drop the first user.start and stall the workflow.
        let sub_tx = subscribe_or_log("packet.tx-request");
        let sub_send = subscribe_or_log("send.schedule-request");
        let sub_ar = subscribe_or_log("scheduler.action-result");
        let sub_rx = subscribe_or_log("packet.rx");
        let sub_ctrl = subscribe_or_log("scheduler.control.*");
        let sub_timer = subscribe_or_log("scheduler.timer.*");
        let sub_user = subscribe_or_log("scheduler.user.*");
        let sub_topo = subscribe_or_log("topology.changed");

        // Start pulling RX from host and publishing `packet.rx`.
        // Must happen after subscribing to `packet.rx` to avoid dropping the first RX events.
        spawn_rx_pump();

        // Now safe to publish scheduler.user.start based on the scenario's ramp-up.
        init_runtime(&ctx)?;

        publish_scheduler_state(SchedulerState::Running, None);

        // Long-running loop: block when idle instead of ticking out.
        let loop_result = run_event_loop(
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

/// Pull RX batches from host via `ntx:host/rx-ring@0.1.0` and publish `packet.rx` events.
///
/// This must never panic; errors are logged and the loop continues.
fn spawn_rx_pump() {
    // NOTE: `wit-bindgen` guest components are single-threaded by default, but WASI threads
    // may be available depending on runtime. We keep this as best-effort:
    // if thread spawn fails, we fall back to no RX pumping (the rest of scheduler still runs).
    let _ = std::thread::Builder::new()
        .name("rx-pump".to_string())
        .spawn(|| loop {
            // Stop flag
            if RUNTIME.lock().map(|rt| rt.stop).unwrap_or(false) {
                return;
            }

            // Use wait to avoid busy-loop. Timeout keeps us responsive to stop flag.
            let batch = ntx::host::rx_ring::wait_rx(64 * 1024, 256 * 1024, 50);
            let Some(batch) = batch else {
                continue;
            };

            // Always release handle.
            let handle = batch.handle;

            // Read full buffers currently (simple + correct). We can optimize to slice reads later.
            // Any read error must not panic.
            let desc_mem = match ntx::host::rx_ring::read_desc(handle, 0, batch.desc_len) {
                Ok(v) => v,
                Err(e) => {
                    println!("[scheduler] rx-ring read-desc failed: {e}");
                    let _ = ntx::host::rx_ring::release(handle);
                    continue;
                }
            };
            let payload_mem = match ntx::host::rx_ring::read_payload(handle, 0, batch.payload_len) {
                Ok(v) => v,
                Err(e) => {
                    println!("[scheduler] rx-ring read-payload failed: {e}");
                    let _ = ntx::host::rx_ring::release(handle);
                    continue;
                }
            };

            // Drain and publish packet.rx events.
            let _ = crate::rx_decode::drain_rx_ring(desc_mem, payload_mem);

            let _ = ntx::host::rx_ring::release(handle);
        });
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SchedulerState {
    Idle,
    Running,
    Completed,
    Error,
}

static SCHED_STATE: Lazy<Mutex<SchedulerState>> = Lazy::new(|| Mutex::new(SchedulerState::Idle));

/// 强类型配置结构（最小版）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Scenario {
    #[serde(default)]
    workbook: Workbook,
    #[serde(default)]
    actions: Actions,
    #[serde(default)]
    workflows: Workflow,
    #[serde(default)]
    load: Load,
    #[serde(default)]
    user_resources: UserResources,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Workbook {
    #[serde(default)]
    resources: Vec<Resource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Resource {
    id: String,
    #[serde(rename = "type")]
    r#type: String,
    #[serde(default)]
    properties: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Actions {
    #[serde(default)]
    actions: Vec<Action>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Action {
    id: String,
    call: String,
    #[serde(default)]
    with: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Workflow {
    #[serde(default)]
    nodes: Vec<WorkflowNodeDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkflowNodeDef {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    action: Option<String>,
    /// 多 step action：同一 node 内按顺序执行 actions，全部成功后才沿边推进。
    /// 兼容：若提供 actions，则优先使用；否则使用 action。
    #[serde(default)]
    actions: Option<Vec<String>>,
    /// 更强的 step 语义：每个 step 可覆写 retry/timeout，并支持失败/超时跳转到指定 step。
    /// 若提供 steps，则优先使用 steps；否则退化为 actions/action。
    #[serde(default)]
    steps: Option<Vec<NodeStepDef>>,
    /// 调度优先级（越大越优先）；默认 0。
    #[serde(default)]
    priority: Option<i32>,
    /// wait 节点：等待的事件与匹配条件（最小支持 packet.rx + action_id 匹配）
    #[serde(default)]
    on: Option<WaitOnSpec>,
    #[serde(default)]
    edges: Vec<WorkflowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeStepDef {
    action: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    retry: Option<RetryDef>,
    #[serde(default)]
    on_failed_step: Option<u32>,
    #[serde(default)]
    on_timeout_step: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RetryDef {
    #[serde(default)]
    max: i64,
    #[serde(default)]
    backoff_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WaitOnSpec {
    event: String,
    #[serde(default)]
    r#match: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkflowEdge {
    to: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    trigger: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Load {
    #[serde(default)]
    ramp_up: RampUp,
    #[serde(default)]
    user_lifetime: UserLifetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RampUp {
    #[serde(default)]
    phases: Vec<RampPhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RampPhase {
    at_second: u64,
    spawn_users: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UserLifetime {
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default)]
    iterations: Option<u64>,
    #[serde(default)]
    think_time: Option<String>,
    /// 每个 user 的并发上限（Running task 数）；默认 1。
    #[serde(default)]
    max_concurrency: Option<u32>,
}

fn default_mode() -> String {
    "once".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UserResources {
    #[serde(default)]
    ip_binding: Option<IpBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IpBinding {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    pool_id: Option<String>,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default)]
    release_on: Option<String>,
}

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

/// StateMachine（方案B）：权威的 workflow 引擎（per-user task 状态 + 边推进）。
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
                            publish_event(
                                "scheduler.task.state-changed",
                                Some(&user_id),
                                Some(&node_id),
                                None,
                                serde_json::json!({
                                    "from": format!("{:?}", prev),
                                    "to": format!("{:?}", state),
                                    "scenario_version": u.meta.scenario_version,
                                    "ts_ms": now_ms(),
                                }),
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

fn load_scenario_config(config_dir: &str) -> Result<ScenarioConfig, String> {
    let meta =
        fs::metadata(config_dir).map_err(|e| format!("check config dir {config_dir}: {e}"))?;
    if !meta.is_dir() {
        return Err(format!("config dir is not a directory: {config_dir}"));
    }

    let workflow_raw = read_optional_file(config_dir, "workflow.yaml")
        .or_else(|_| read_optional_file(config_dir, "workflow.json"))
        .ok();
    let workbook_raw = read_optional_file(config_dir, "workbook.yaml")
        .or_else(|_| read_optional_file(config_dir, "workbook.json"))
        .ok();
    let actions_raw = read_optional_file(config_dir, "actions.yaml")
        .or_else(|_| read_optional_file(config_dir, "actions.json"))
        .ok();
    let load_raw = read_optional_file(config_dir, "load.yaml")
        .or_else(|_| read_optional_file(config_dir, "load.json"))
        .ok();

    let mut cfg = ScenarioConfig {
        config_dir: config_dir.to_string(),
        workflow_raw,
        workbook_raw,
        actions_raw,
        load_raw,
        parsed: None,
    };

    cfg.parsed = parse_scenario(config_dir, &cfg)?;
    Ok(cfg)
}

fn read_optional_file(dir: &str, name: &str) -> Result<String, String> {
    let path = format!("{}/{}", dir, name);
    let content = fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
    println!("[scheduler] loaded config file: {path}");
    Ok(content)
}

fn log_config_summary(cfg: &ScenarioConfig) -> Result<(), String> {
    let mut buf = String::new();
    writeln!(
        &mut buf,
        "[scheduler] config summary dir={} workflow={} workbook={} actions={} load={}",
        cfg.config_dir,
        cfg.workflow_raw.as_ref().map(|s| s.len()).unwrap_or(0),
        cfg.workbook_raw.as_ref().map(|s| s.len()).unwrap_or(0),
        cfg.actions_raw.as_ref().map(|s| s.len()).unwrap_or(0),
        cfg.load_raw.as_ref().map(|s| s.len()).unwrap_or(0)
    )
    .map_err(|e| format!("format summary: {e}"))?;
    print!("{buf}");
    Ok(())
}

/// 解析 scenario（优先 scenario.yaml/json；否则分文件合并）
fn parse_scenario(config_dir: &str, raw: &ScenarioConfig) -> Result<Option<Scenario>, String> {
    let scenario_file_yaml = format!("{}/scenario.yaml", config_dir);
    let scenario_file_json = format!("{}/scenario.json", config_dir);
    if let Ok(content) = fs::read_to_string(&scenario_file_yaml) {
        let sc: Scenario = serde_yaml::from_str(&content)
            .or_else(|_| serde_json::from_str(&content))
            .map_err(|e| format!("parse scenario.yaml: {e}"))?;
        validate_scenario(&sc)?;
        return Ok(Some(sc));
    }
    if let Ok(content) = fs::read_to_string(&scenario_file_json) {
        let sc: Scenario =
            serde_json::from_str(&content).map_err(|e| format!("parse scenario.json: {e}"))?;
        validate_scenario(&sc)?;
        return Ok(Some(sc));
    }

    if raw.workflow_raw.is_none() && raw.workbook_raw.is_none() && raw.actions_raw.is_none() {
        return Ok(None);
    }

    let workflow: Workflow = parse_piece(raw.workflow_raw.as_ref(), "workflow")?;
    let workbook: Workbook = parse_piece(raw.workbook_raw.as_ref(), "workbook")?;
    let actions: Actions = parse_piece(raw.actions_raw.as_ref(), "actions")?;
    let load: Load = parse_piece(raw.load_raw.as_ref(), "load").unwrap_or_default();
    let user_resources: UserResources = parse_piece(None, "user_resources").unwrap_or_default();

    let sc = Scenario {
        workbook,
        actions,
        workflows: workflow,
        load,
        user_resources,
    };
    validate_scenario(&sc)?;
    Ok(Some(sc))
}

fn parse_piece<T: for<'de> Deserialize<'de> + Default>(
    raw: Option<&String>,
    name: &str,
) -> Result<T, String> {
    if let Some(text) = raw {
        serde_yaml::from_str::<T>(text)
            .or_else(|_| serde_json::from_str::<T>(text))
            .map_err(|e| format!("parse {name}: {e}"))
    } else {
        Ok(T::default())
    }
}

fn validate_scenario(sc: &Scenario) -> Result<(), String> {
    let mut action_ids = HashMap::new();
    for a in &sc.actions.actions {
        if action_ids.insert(&a.id, ()).is_some() {
            return Err(format!("duplicate action id: {}", a.id));
        }
    }

    let mut resource_ids = HashMap::new();
    for r in &sc.workbook.resources {
        if resource_ids.insert(&r.id, ()).is_some() {
            return Err(format!("duplicate resource id: {}", r.id));
        }
    }

    for n in &sc.workflows.nodes {
        if action_ids.is_empty() && n.action.is_some() {
            // allow empty
        }
    }

    let mut node_ids = HashMap::new();
    for n in &sc.workflows.nodes {
        if node_ids.insert(&n.id, ()).is_some() {
            return Err(format!("duplicate workflow node id: {}", n.id));
        }
        if n.kind == "action" {
            let has_steps = n.steps.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
            if has_steps {
                for st in n.steps.as_ref().unwrap() {
                    if !action_ids.contains_key(&st.action) {
                        return Err(format!(
                            "workflow node {} references unknown action {}",
                            n.id, st.action
                        ));
                    }
                }
            } else {
                let has_actions = n.actions.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
                if has_actions {
                    for aid in n.actions.as_ref().unwrap() {
                        if !action_ids.contains_key(aid) {
                            return Err(format!(
                                "workflow node {} references unknown action {}",
                                n.id, aid
                            ));
                        }
                    }
                } else if let Some(action_id) = &n.action {
                    if !action_ids.contains_key(action_id) {
                        return Err(format!(
                            "workflow node {} references unknown action {}",
                            n.id, action_id
                        ));
                    }
                } else {
                    return Err(format!(
                        "workflow node {} is type=action but missing steps/actions/action",
                        n.id
                    ));
                }
            }
        } else if let Some(action_id) = &n.action {
            // allow legacy/extra fields, but validate if provided
            if !action_ids.contains_key(action_id) {
                return Err(format!(
                    "workflow node {} references unknown action {}",
                    n.id, action_id
                ));
            }
        }
    }
    for n in &sc.workflows.nodes {
        for e in &n.edges {
            if !node_ids.contains_key(&e.to) {
                return Err(format!(
                    "workflow edge from {} to missing node {}",
                    n.id, e.to
                ));
            }
        }
    }

    // Note: ip_binding.pool_id is a host-side resource pool *name* (e.g. "default"),
    // not a workbook resource id. We only do best-effort validation here.
    Ok(())
}

// ----------------- 运行态初始化与事件循环 -----------------

fn subscribe_or_log(filter: &str) -> Option<String> {
    match ntx::scenario_eventbus::event_bus::subscribe(filter) {
        Ok(id) => {
            println!("[scheduler] subscribed {} -> {}", filter, id);
            Some(id)
        }
        Err(e) => {
            println!("[scheduler] subscribe {} failed: {}", filter, e);
            None
        }
    }
}

fn init_runtime(ctx: &SchedulerContext) -> Result<(), String> {
    let Some(sc) = &ctx.scenario.parsed else {
        return Ok(());
    };

    // reset runtime flags
    if let Ok(mut rt) = RUNTIME.lock() {
        rt.users.clear();
        rt.ready.clear();
        rt.paused = false;
        rt.stop = false;
    }

    // init load controller
    if let Ok(mut lc) = LOAD.lock() {
        lc.started_at_ms = now_ms();
        lc.next_phase = 0;
        lc.next_user_seq = 1;
        lc.phases = sc.load.ramp_up.phases.clone();
    }

    // init scenario registry with version=1
    if let Ok(mut reg) = SCENARIOS.lock() {
        reg.reset_with(sc.clone());
    }

    // 若没有 ramp-up phases，默认立即启动 1 个用户
    if sc.load.ramp_up.phases.is_empty() {
        publish_user_start_event(now_ms(), 1, None);
    } else {
        // 允许 phase.at_second == 0 立即触发
        tick_load_controller(now_ms());
    }

    Ok(())
}

fn build_resources_json(sc: &Scenario) -> serde_json::Value {
    let mut resources = serde_json::Map::new();
    for r in &sc.workbook.resources {
        resources.insert(r.id.clone(), r.properties.clone());
    }
    serde_json::json!({
        "resource": serde_json::Value::Object(resources),
        "_bound": {},
    })
}

/// 简易事件循环：轮询多个订阅，处理 packet.rx / packet.tx-request / scheduler.action-result，并调度 ready tasks。
fn run_event_loop(
    ctx: &SchedulerContext,
    sub_tx: Option<&str>,
    sub_send: Option<&str>,
    sub_ar: Option<&str>,
    sub_rx: Option<&str>,
    sub_ctrl: Option<&str>,
    sub_timer: Option<&str>,
    sub_user: Option<&str>,
    sub_topo: Option<&str>,
) -> Result<(), String> {
    let mut idle = 0u32;
    loop {
        let mut did_work = false;

        // 1) control events (blocking wait)
        if let Some(id) = sub_ctrl {
            let events = ntx::scenario_eventbus::event_bus::wait_events(id, 64, 50)
                .map_err(|e| format!("poll_events(control): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                handlers::on_control_event(&ev);
            }
        }

        // stop flag
        if RUNTIME.lock().map(|rt| rt.stop).unwrap_or(false) {
            return Ok(());
        }

        if let Some(id) = sub_tx {
            let events = ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(packet.tx-request): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "packet.tx-request" {
                    if let Err(e) = tx::handle_tx_request(&ev.payload, ev.correlation_id.as_deref())
                    {
                        println!("[scheduler] process tx-request failed: {e}");
                    }
                }
            }
        }

        // 1.5) send schedule requests (executor -> scheduler)
        if let Some(id) = sub_send {
            let events = ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(send.schedule-request): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "send.schedule-request" {
                    if let Err(e) = send_scheduler::on_send_schedule_request(&ev) {
                        println!("[scheduler] handle send.schedule-request failed: {e}");
                    }
                }
            }
        }

        // 2) action-result events (async-compatible)
        if let Some(id) = sub_ar {
            let events = ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(action-result): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "scheduler.action-result" {
                    action_result::on_action_result_event(ctx, &ev)?;
                }
            }
        }

        if let Some(id) = sub_rx {
            let events = ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(packet.rx): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "packet.rx" {
                    handlers::on_packet_rx(ctx, &ev)?;
                }
            }
        }

        // 3) timer events
        if let Some(id) = sub_timer {
            let events = ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(timer): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind.starts_with("scheduler.timer.") {
                    handlers::on_timer_event(ctx, &ev)?;
                }
            }
        }

        // 4) user lifecycle events
        if let Some(id) = sub_user {
            let events = ntx::scenario_eventbus::event_bus::wait_events(id, 64, 0)
                .map_err(|e| format!("poll_events(user): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                match ev.kind.as_str() {
                    "scheduler.user.start" => handlers::on_user_start_event(ctx, &ev)?,
                    "scheduler.user.exit" => handlers::on_user_exit_event(ctx, &ev)?,
                    _ => {}
                }
            }
        }

        // 4.1) topology change events (affect only NEW users)
        if let Some(id) = sub_topo {
            let events = ntx::scenario_eventbus::event_bus::wait_events(id, 16, 0)
                .map_err(|e| format!("poll_events(topology): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "topology.changed" {
                    if let Err(e) = handlers::on_topology_changed_event(ctx, &ev) {
                        // on_topology_changed_event already published scheduler.topology.rejected with rich payload
                        println!("[scheduler] warn: topology.changed rejected: {}", e);
                    }
                }
            }
        }

        // 5) drive load/timers
        let now = now_ms();
        tick_load_controller(now);
        tick_timers(now);

        let paused = RUNTIME.lock().map(|rt| rt.paused).unwrap_or(false);
        if !paused {
            // 调度 ready tasks（最小实现：每 tick 尝试跑一批）
            did_work |= dispatch_ready_tasks(ctx, 16)?;
        }

        // send-scheduler tick：检查到期的 send-request 并发包
        send_scheduler::tick_send_scheduler(now_ms());
        // best-effort cleanup
        cleanup_sock_ctx(now_ms(), 60_000);

        if did_work {
            idle = 0;
        } else {
            idle += 1;
        }

        if idle >= 10 {
            let has_active_send = send_scheduler::has_active_jobs();
            let has_ready = RUNTIME
                .lock()
                .map(|rt| !rt.ready.is_empty())
                .unwrap_or(false);
            if !has_active_send && !has_ready {
                // No useful work; keep blocking instead of exiting.
                // We'll wait via the control subscription above.
                idle = 0;
            }
        }
    }
}

fn maybe_finish_user(_ctx: &SchedulerContext, user_id: &str) -> Result<(), String> {
    let (_ver, sc_arc, _wf_idx) = get_user_scenario_ctx(user_id)?;
    let sc = sc_arc.as_ref();
    let reached_end = {
        let sm = STATE_MACHINE
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

fn publish_user_exit_event(user_id: &str, reason: &str) {
    let id = format!("ux-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed));
    let payload = serde_json::json!({ "user_id": user_id, "reason": reason }).to_string();
    let _ = ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
        id,
        kind: "scheduler.user.exit".to_string(),
        user_id: Some(user_id.to_string()),
        task_id: None,
        action_id: None,
        payload,
        correlation_id: None,
        timestamp_ms: now_ms(),
    });
}

fn restart_user_iteration(_ctx: &SchedulerContext, user_id: &str) -> Result<(), String> {
    let (_ver, sc_arc, _wf_idx) = get_user_scenario_ctx(user_id)?;
    restart_user_iteration_with_scenario(sc_arc.as_ref(), user_id)
}

fn restart_user_iteration_with_scenario(sc: &Scenario, user_id: &str) -> Result<(), String> {
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
        let mut sm = STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        sm.apply(
            sc,
            &wf_idx,
            now_ms(),
            SmEvent::UserReset {
                user_id: user_id.to_string(),
            },
        )
    };
    apply_sm_effects(effects)?;
    Ok(())
}

fn user_meta_from_config(ul: &UserLifetime) -> UserMeta {
    let mode = ul.mode.trim().to_ascii_lowercase();
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

fn cleanup_sock_ctx(now_ms: u64, max_age_ms: u64) {
    if let Ok(mut map) = SOCK_CTX.lock() {
        map.retain(|_, ctx| now_ms.saturating_sub(ctx.last_seen_ms) <= max_age_ms);
    }
}

fn publish_scheduler_state(state: SchedulerState, err: Option<&String>) {
    if let Ok(mut st) = SCHED_STATE.lock() {
        *st = state;
    }
    let payload = serde_json::json!({
        "state": format!("{:?}", state),
        "error": err.cloned(),
    })
    .to_string();
    let id = format!("ss-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed));
    let _ = ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
        id,
        kind: "scheduler.state-changed".to_string(),
        user_id: None,
        task_id: None,
        action_id: None,
        payload,
        correlation_id: None,
        timestamp_ms: now_ms(),
    });
}

fn dispatch_ready_tasks(ctx: &SchedulerContext, max: usize) -> Result<bool, String> {
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
        let (_ver, sc_arc, wf_idx) = match get_user_scenario_ctx(&user_id) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let sc = sc_arc.as_ref();

        // 找 node
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
            action.call
        );

        // B) 真实资源绑定：为 udp action 确保 user 已绑定 socket，并注入 socket_id
        if action.call.starts_with("udp.") {
            ensure_udp_socket_for_user(ctx, sc, &user_id)?;
        }

        // per-user concurrency cap + 状态机 Ready->Running
        let (task_vars, task_exports, user_resources) = {
            // 1) concurrency check (runtime meta)
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
            // 2) state-machine transition (authoritative) via apply(DispatchStart)
            let effects = {
                let mut sm = STATE_MACHINE
                    .lock()
                    .map_err(|_| "lock state-machine".to_string())?;
                sm.apply(
                    sc,
                    &wf_idx,
                    now_ms(),
                    SmEvent::DispatchStart {
                        user_id: user_id.to_string(),
                        node_id: node_id.to_string(),
                    },
                )
            };
            if effects.is_empty() {
                // stale ready item
                eprintln!("[scheduler] dispatch: stale_ready user_id={user_id} node_id={node_id}");
                continue;
            }
            apply_sm_effects(effects)?;
            // 3) write derived state to runtime + snapshot context
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

        // 初始化 retry policy（step 覆写优先；否则来自 action.with.retry.max/backoff_ms）
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
        if action.call.starts_with("udp.") {
            inject_udp_socket_id(&user_id, &mut def)?;
        }

        // 超时 timer：step.timeout_ms 覆写优先；否则 action.with.timeout-ms/timeout_ms（毫秒）
        let timeout_ms = step_timeout_ms.or_else(|| {
            action
                .with
                .get("timeout-ms")
                .or_else(|| action.with.get("timeout_ms"))
                .and_then(|v| v.as_u64())
        });
        if let Some(tmo) = timeout_ms {
            schedule_timer(
                "scheduler.timer.timeout",
                now_ms().saturating_add(tmo),
                &user_id,
                &node_id,
                Some(&def.id),
                serde_json::json!({"user_id": user_id, "task_id": node_id, "action_id": def.id}),
            );
        }

        eprintln!(
            "[scheduler] dispatch: execute_action user_id={user_id} node_id={node_id} action_id={} corr_id={}",
            def.id,
            act_ctx
                .correlation_id
                .as_deref()
                .unwrap_or("<none>")
        );

        let outcome =
            ntx::scenario_actions_executor::action_component::execute_action(&def, Some(&act_ctx))
                .map_err(|e| format!("execute_action failed: {e}"))?;

        eprintln!(
            "[scheduler] dispatch: action_outcome user_id={user_id} node_id={node_id} action_id={} status={:?}",
            def.id, outcome.status
        );

        // 结果事件化：发布 action-result，由事件处理器更新状态/重试/超时（兼容 future async）
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

fn publish_action_result_event(
    user_id: &str,
    task_id: &str,
    action_id: &str,
    correlation_id: Option<&str>,
    outcome: &ActionOutcome,
) -> Result<(), String> {
    let now = now_ms();
    let metrics = outcome.metrics.as_ref().map(|m| {
        serde_json::json!({
            "latency_ms": m.latency_ms,
            "bytes_sent": m.bytes_sent,
            "bytes_received": m.bytes_received,
            "response_code": m.response_code,
        })
    });
    let payload = serde_json::json!({
        "status": format!("{:?}", outcome.status),
        "detail": outcome.detail,
        "metrics": metrics,
        "exports": outcome.exports,
    })
    .to_string();

    let id = format!("ar-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed));
    ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
        id,
        kind: "scheduler.action-result".to_string(),
        user_id: Some(user_id.to_string()),
        task_id: Some(task_id.to_string()),
        action_id: Some(action_id.to_string()),
        payload,
        correlation_id: correlation_id.map(|s| s.to_string()),
        timestamp_ms: now,
    })
    .map_err(|e| format!("publish scheduler.action-result: {e}"))?;
    Ok(())
}

// ----------------- B) 真实资源绑定（最小：UDP socket create+bind） -----------------

fn ensure_udp_socket_for_user(
    _ctx: &SchedulerContext,
    sc: &Scenario,
    user_id: &str,
) -> Result<(), String> {
    // already bound?
    if let Ok(rt) = RUNTIME.lock() {
        if let Some(u) = rt.users.get(user_id) {
            if get_bound_udp_socket_id(&u.resources).is_some() {
                return Ok(());
            }
        }
    }

    // pick first udp-endpoint resource
    let res = sc
        .workbook
        .resources
        .iter()
        .find(|r| r.r#type == "udp-endpoint")
        .or_else(|| {
            sc.workbook
                .resources
                .iter()
                .find(|r| r.r#type == "udp-target")
        })
        .ok_or_else(|| "no udp-endpoint resource in workbook.resources".to_string())?;

    let p = &res.properties;
    let peer_ip = parse_ipv4(p, &["peer_ip", "peer-ip", "peer_ipv4", "peer-ipv4"])
        .ok_or_else(|| "missing peer_ip".to_string())?;
    let peer_mac = parse_mac(
        p,
        &["peer_mac", "peer-mac", "peer_mac_addr", "peer-mac-addr"],
    );
    let peer_port =
        parse_u16(p, &["peer_port", "peer-port"]).ok_or_else(|| "missing peer_port".to_string())?;

    let ttl = p
        .get("ttl")
        .and_then(|v| v.as_u64())
        .and_then(|v| u8::try_from(v).ok());

    // create + bind
    let sock = udp_socket_control::create(user_id)
        .map_err(|e| format!("udp_socket_control.create: {:?}", e))?;

    // pool name: prefer scenario.user_resources.ip_binding.pool_id; then resource.properties.pool; else "default"
    let pool = sc
        .user_resources
        .ip_binding
        .as_ref()
        .and_then(|b| b.pool_id.clone())
        .or_else(|| {
            p.get("pool")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "default".to_string());

    let ident = resources::acquire_udp_identity(&pool, &sock.owner)
        .map_err(|e| format!("resources.acquire_udp_identity(pool={pool}): {:?}", e))?;

    // peer mac may be resolved by host (best-effort) if not configured
    let peer_mac = match peer_mac {
        Some(m) => m,
        None => {
            let mac = resources::resolve_peer_mac(to_wit_ipv4(peer_ip)).map_err(|e| {
                format!("resources.resolve_peer_mac(peer_ip={:?}): {:?}", peer_ip, e)
            })?;
            [mac.a, mac.b, mac.c, mac.d, mac.e, mac.f]
        }
    };

    let bind = udp_socket_control::UdpBind {
        local_ipv4: ident.local_ipv4,
        local_mac: ident.local_mac,
        local_udp_port: ident.local_udp_port,
        peer_ipv4: to_wit_ipv4(peer_ip),
        peer_port,
        peer_mac: to_wit_mac(peer_mac),
        ttl,
    };

    udp_socket_control::bind(sock.sock, bind)
        .map_err(|e| format!("udp_socket_control.bind: {:?}", e))?;

    // store binding into runtime resources
    if let Ok(mut rt) = RUNTIME.lock() {
        if let Some(u) = rt.users.get_mut(user_id) {
            set_bound_udp_socket_id(&mut u.resources, sock.sock);
            set_bound_udp_owner_id(&mut u.resources, &sock.owner);
        }
    }

    // best-effort: also publish a small event for observability
    let _ = ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
        id: format!("sock-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)),
        kind: "scheduler.resource-bound".to_string(),
        user_id: Some(user_id.to_string()),
        task_id: None,
        action_id: None,
        payload: serde_json::json!({"resource": res.id, "sock_id": sock.sock}).to_string(),
        correlation_id: None,
        timestamp_ms: now_ms(),
    });

    Ok(())
}

fn inject_udp_socket_id(user_id: &str, def: &mut ActionDef) -> Result<(), String> {
    udp_binding::inject_udp_socket_id(user_id, def)
}

fn get_bound_udp_socket_id(resources: &serde_json::Value) -> Option<u64> {
    udp_binding::get_bound_udp_socket_id(resources)
}

fn get_bound_udp_owner_id(resources: &serde_json::Value) -> Option<String> {
    udp_binding::get_bound_udp_owner_id(resources)
}

fn set_bound_udp_socket_id(resources: &mut serde_json::Value, sock_id: u64) {
    udp_binding::set_bound_udp_socket_id(resources, sock_id)
}

fn set_bound_udp_owner_id(resources: &mut serde_json::Value, owner_id: &str) {
    udp_binding::set_bound_udp_owner_id(resources, owner_id)
}

fn finish_user(_ctx: &SchedulerContext, user_id: &str) -> Result<(), String> {
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
                let _ = ntx::scenario_eventbus::event_bus::publish(
                    &ntx::scenario_eventbus::event_bus::Event {
                        id: format!("rr-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)),
                        kind: "scheduler.resource-released".to_string(),
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

fn parse_ipv4(props: &serde_json::Value, keys: &[&str]) -> Option<[u8; 4]> {
    let v = keys.iter().find_map(|k| props.get(*k))?;
    if let Some(s) = v.as_str() {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 4 {
            return None;
        }
        let mut out = [0u8; 4];
        for (i, p) in parts.iter().enumerate() {
            out[i] = p.parse::<u8>().ok()?;
        }
        return Some(out);
    }
    if let Some(arr) = v.as_array() {
        if arr.len() != 4 {
            return None;
        }
        let mut out = [0u8; 4];
        for i in 0..4 {
            out[i] = arr[i].as_u64().and_then(|n| u8::try_from(n).ok())?;
        }
        return Some(out);
    }
    None
}

fn parse_mac(props: &serde_json::Value, keys: &[&str]) -> Option<[u8; 6]> {
    let v = keys.iter().find_map(|k| props.get(*k))?;
    if let Some(s) = v.as_str() {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 6 {
            return None;
        }
        let mut out = [0u8; 6];
        for i in 0..6 {
            out[i] = u8::from_str_radix(parts[i], 16).ok()?;
        }
        return Some(out);
    }
    if let Some(arr) = v.as_array() {
        if arr.len() != 6 {
            return None;
        }
        let mut out = [0u8; 6];
        for i in 0..6 {
            out[i] = arr[i].as_u64().and_then(|n| u8::try_from(n).ok())?;
        }
        return Some(out);
    }
    None
}

fn parse_u16(props: &serde_json::Value, keys: &[&str]) -> Option<u16> {
    let v = keys.iter().find_map(|k| props.get(*k))?;
    if let Some(n) = v.as_u64() {
        return u16::try_from(n).ok();
    }
    if let Some(s) = v.as_str() {
        return s.parse::<u16>().ok();
    }
    None
}

fn to_wit_ipv4(ip: [u8; 4]) -> types::Ipv4Addr {
    types::Ipv4Addr {
        a: ip[0],
        b: ip[1],
        c: ip[2],
        d: ip[3],
    }
}

fn to_wit_mac(mac: [u8; 6]) -> types::MacAddr {
    types::MacAddr {
        a: mac[0],
        b: mac[1],
        c: mac[2],
        d: mac[3],
        e: mac[4],
        f: mac[5],
    }
}

// ----------------- 模板展开与 Action 构造 -----------------
#[allow(dead_code)]
fn build_action_def_with_ctx(
    action: &Action,
    ctx: &TemplateContext,
    user_id: Option<&str>,
    task_id: Option<&str>,
) -> Result<(ActionDef, ActionContext), String> {
    let expanded = template::render_value(&action.with, ctx)?;
    let params = serde_json::to_string(&expanded).map_err(|e| format!("encode params: {e}"))?;

    let def = ActionDef {
        id: action.id.clone(),
        call: action.call.clone(),
        params,
        exports: vec![], // 预留 exports，后续从配置补充
    };

    let act_ctx = ActionContext {
        user_id: user_id.map(|s| s.to_string()),
        task_id: task_id.map(|s| s.to_string()),
        action_id: Some(action.id.clone()),
        // Correlate: one id per action invocation for tracing across tx/rx/action-result.
        correlation_id: Some(format!(
            "corr-{}",
            EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)
        )),
        vars: Some(ctx.vars.to_string()),
        resources: Some(ctx.resources.to_string()),
        deadline_ms: None,
    };

    Ok((def, act_ctx))
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
    time::now_ms()
}
