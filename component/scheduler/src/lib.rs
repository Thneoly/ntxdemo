//! 调度器组件骨架（wasm32-wasip2）。
//! 仅提供占位实现，便于后续对接状态机与负载控制逻辑。

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
struct SchedulerExports;

impl exports::ntx::scenario_scheduler::scheduler_component::Guest for SchedulerExports {
    fn run(config_dir: String) -> Result<(), String> {
        println!("[scheduler] run with config dir: {config_dir}");

        let scenario = load_scenario_config(&config_dir)?;
        log_config_summary(&scenario)?;
        let ctx = SchedulerContext { scenario };
        init_runtime(&ctx)?;

        let sub_tx = subscribe_or_log("packet.tx-request");
        let sub_send = subscribe_or_log("send.schedule-request");
        let sub_ar = subscribe_or_log("scheduler.action-result");
        let sub_rx = subscribe_or_log("packet.rx");
        let sub_ctrl = subscribe_or_log("scheduler.control.*");
        let sub_timer = subscribe_or_log("scheduler.timer.*");
        let sub_user = subscribe_or_log("scheduler.user.*");
        let sub_topo = subscribe_or_log("topology.changed");

        publish_scheduler_state(SchedulerState::Running, None);

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
            256,
        );

        match &loop_result {
            Ok(_) => publish_scheduler_state(SchedulerState::Completed, None),
            Err(e) => publish_scheduler_state(SchedulerState::Error, Some(e)),
        }

        if let Some(id) = sub_tx {
            let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&id);
        }
        if let Some(id) = sub_send {
            let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&id);
        }
        if let Some(id) = sub_ar {
            let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&id);
        }
        if let Some(id) = sub_rx {
            let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&id);
        }
        if let Some(id) = sub_ctrl {
            let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&id);
        }
        if let Some(id) = sub_timer {
            let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&id);
        }
        if let Some(id) = sub_user {
            let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&id);
        }
        if let Some(id) = sub_topo {
            let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&id);
        }
        loop_result
    }
}

// impl exports::ntx::scenario_send_scheduler::send_scheduler::Guest for SchedulerExports {
//     fn schedule_send(request: SendRequest) -> Result<String, String> {
//         schedule_send_job(request)
//     }

// fn cancel_send(request_id: String) -> Result<(), String> {
//     cancel_send_job(&request_id)
// }

// fn query_send_status(request_id: String) -> Result<SendStatus, String> {
//     query_send_status_job(&request_id)
// }
// }

impl exports::ntx::scenario_scheduler::packet_ingest::Guest for SchedulerExports {
    fn notify_rx(desc_mem: Vec<u8>, payload_mem: Vec<u8>) -> Result<u32, String> {
        Ok(drain_rx_ring(desc_mem, payload_mem))
    }
}

impl exports::ntx::scenario_scheduler::packet_tx::Guest for SchedulerExports {
    fn process_tx_request(payload_json: String) -> Result<(), String> {
        // host direct call: no correlation_id available
        handle_tx_request(&payload_json, None)
    }
}

export!(SchedulerExports);

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_yaml;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt::Write;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const NTX_MAGIC: u32 = 0x4E54_5830; // "NTX0"
const NTX_VERSION: u16 = 1;
const CONTROL_LEN: usize = 48;
const DESC_LEN: usize = 32;
const DESCS_OFF: usize = 0x1000;
const MAX_CONSUME: u32 = 64;

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);
static PACKET_RX_SEQ: AtomicU64 = AtomicU64::new(1);

fn publish_event(
    kind: &str,
    user_id: Option<&str>,
    task_id: Option<&str>,
    action_id: Option<&str>,
    payload: serde_json::Value,
) {
    let _ = ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
        id: format!("ev-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)),
        kind: kind.to_string(),
        user_id: user_id.map(|s| s.to_string()),
        task_id: task_id.map(|s| s.to_string()),
        action_id: action_id.map(|s| s.to_string()),
        payload: payload.to_string(),
        correlation_id: None,
        timestamp_ms: now_ms(),
    });
}

fn publish_event_with_corr(
    kind: &str,
    user_id: Option<&str>,
    task_id: Option<&str>,
    action_id: Option<&str>,
    correlation_id: Option<&str>,
    payload: serde_json::Value,
) {
    let _ = ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
        id: format!("ev-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)),
        kind: kind.to_string(),
        user_id: user_id.map(|s| s.to_string()),
        task_id: task_id.map(|s| s.to_string()),
        action_id: action_id.map(|s| s.to_string()),
        payload: payload.to_string(),
        correlation_id: correlation_id.map(|s| s.to_string()),
        timestamp_ms: now_ms(),
    });
}

/// 配置占位：后续将解析 workflow / workbook / load。
#[derive(Clone, Debug, Default)]
struct ScenarioConfig {
    config_dir: String,
    workflow_raw: Option<String>,
    workbook_raw: Option<String>,
    actions_raw: Option<String>,
    load_raw: Option<String>,

    parsed: Option<Scenario>,
}

/// 运行期上下文，占位后续扩展。
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct SchedulerContext {
    scenario: ScenarioConfig,
}

/// 运行态的 User/Task/Ready 队列（极简版）
#[derive(Default)]
struct RuntimeState {
    users: HashMap<String, UserInstance>,
    ready: ReadyQueues, // priority queues of (user_id, node_id)
    paused: bool,
    stop: bool,
}

/// 多级就绪队列：按 priority 分桶，优先取更大的 priority。
#[derive(Default)]
struct ReadyQueues {
    by_prio: BTreeMap<i32, VecDeque<(String, String)>>,
}

impl ReadyQueues {
    fn clear(&mut self) {
        self.by_prio.clear();
    }

    fn is_empty(&self) -> bool {
        self.by_prio.values().all(|q| q.is_empty())
    }

    fn push(&mut self, prio: i32, user_id: String, node_id: String) {
        self.by_prio
            .entry(prio)
            .or_insert_with(VecDeque::new)
            .push_back((user_id, node_id));
    }

    fn pop_next(&mut self) -> Option<(String, String)> {
        // pick highest priority with non-empty queue
        let prio = self
            .by_prio
            .iter()
            .rev()
            .find(|(_, q)| !q.is_empty())
            .map(|(p, _)| *p)?;
        let q = self.by_prio.get_mut(&prio)?;
        let item = q.pop_front();
        if q.is_empty() {
            // keep map small
            self.by_prio.remove(&prio);
        }
        item
    }

    fn retain_user(&mut self, user_id: &str) {
        let mut empty_keys: Vec<i32> = Vec::new();
        for (p, q) in self.by_prio.iter_mut() {
            q.retain(|(uid, _)| uid != user_id);
            if q.is_empty() {
                empty_keys.push(*p);
            }
        }
        for k in empty_keys {
            self.by_prio.remove(&k);
        }
    }
}

#[derive(Debug, Clone)]
struct UserInstance {
    tasks: HashMap<String, TaskRuntime>,
    resources: serde_json::Value,
    meta: UserMeta,
}

#[derive(Debug, Clone)]
struct UserMeta {
    mode: String,            // once / loop
    iterations: Option<u64>, // max iterations in loop mode
    think_ms: Option<u64>,   // optional think-time between iterations
    iteration: u64,          // completed iterations
    end_event_sent: bool,    // prevent duplicate exit events per iteration
    running: usize,          // current running tasks
    max_running: usize,      // per-user concurrency cap
    scenario_version: u64,   // topology version bound to this user (old users do not migrate)
}

impl Default for UserMeta {
    fn default() -> Self {
        Self {
            mode: "once".to_string(),
            iterations: None,
            think_ms: None,
            iteration: 0,
            end_event_sent: false,
            running: 0,
            max_running: 1,
            scenario_version: 1,
        }
    }
}

#[derive(Debug, Clone)]
struct TaskRuntime {
    state: TaskState,
    vars: serde_json::Value,
    exports: serde_json::Value,
}

static RUNTIME: Lazy<Mutex<RuntimeState>> = Lazy::new(|| Mutex::new(RuntimeState::default()));

/// Workflow 加速索引（避免每次 packet.rx 扫描全部 wait 节点）
#[derive(Default, Debug, Clone)]
struct WorkflowIndex {
    wait_any: Vec<String>,
    wait_by_action_id: HashMap<String, Vec<String>>,
}

#[derive(Default)]
struct ScenarioRegistry {
    active_version: u64,
    next_version: u64,
    scenarios: HashMap<u64, Arc<Scenario>>,
    wf_index: HashMap<u64, WorkflowIndex>,
}

impl ScenarioRegistry {
    fn reset_with(&mut self, sc: Scenario) {
        self.active_version = 1;
        self.next_version = 2;
        self.scenarios.clear();
        self.wf_index.clear();
        let arc = Arc::new(sc);
        self.wf_index.insert(1, build_workflow_index(&arc));
        self.scenarios.insert(1, arc);
    }

    fn active(&self) -> Option<(u64, Arc<Scenario>, WorkflowIndex)> {
        let v = self.active_version;
        let sc = self.scenarios.get(&v)?.clone();
        let idx = self.wf_index.get(&v).cloned().unwrap_or_default();
        Some((v, sc, idx))
    }

    fn by_version(&self, v: u64) -> Option<(Arc<Scenario>, WorkflowIndex)> {
        let sc = self.scenarios.get(&v)?.clone();
        let idx = self.wf_index.get(&v).cloned().unwrap_or_default();
        Some((sc, idx))
    }

    fn install_new_active(&mut self, sc: Scenario) -> u64 {
        let v = self.next_version.max(1);
        self.next_version = v.saturating_add(1);
        let arc = Arc::new(sc);
        let idx = build_workflow_index(&arc);
        self.scenarios.insert(v, arc);
        self.wf_index.insert(v, idx);
        self.active_version = v;
        v
    }
}

static SCENARIOS: Lazy<Mutex<ScenarioRegistry>> =
    Lazy::new(|| Mutex::new(ScenarioRegistry::default()));

#[derive(Clone, Debug)]
struct TimerJob {
    due_ms: u64,
    kind: String, // event kind, e.g. scheduler.timer.timeout / scheduler.timer.retry
    user_id: Option<String>,
    task_id: Option<String>,
    action_id: Option<String>,
    payload: String, // json string
}

static TIMERS: Lazy<Mutex<Vec<TimerJob>>> = Lazy::new(|| Mutex::new(Vec::new()));

#[derive(serde::Deserialize, Default, Debug, Clone)]
struct PacketRxPayload {
    #[serde(default)]
    sock_id: u64,
    #[serde(default)]
    len: usize,
    #[serde(default)]
    payload_hex: String,
}

#[derive(Clone, Debug, Default)]
struct LoadControllerState {
    started_at_ms: u64,
    next_phase: usize,
    next_user_seq: u64,
    // cached phases
    phases: Vec<RampPhase>,
}

static LOAD: Lazy<Mutex<LoadControllerState>> =
    Lazy::new(|| Mutex::new(LoadControllerState::default()));

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

/// 模板展开上下文（占位）：vars/resources/exports 作为 JSON 对象传入。
#[derive(Debug, Clone, Default)]
struct TemplateContext {
    vars: serde_json::Value,
    resources: serde_json::Value,
    exports: serde_json::Value,
}

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

#[derive(Clone, Debug)]
struct TaskMeta {
    state: TaskState,
    last_update_ms: u64,
    /// action-node 内部的 step index（从 0 开始）。wait/end 节点保持 0。
    step_idx: u32,
}

static SOCK_CTX: Lazy<Mutex<HashMap<u64, SockCtx>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// StateMachine（方案B）：权威的 workflow 引擎（per-user task 状态 + 边推进）。
///
/// 约束：TaskRuntime(vars/exports/resources) 仍在 RUNTIME 内；StateMachine 只负责
/// “哪个节点在什么状态、收到什么事件后如何沿 workflow 边推进”。
#[derive(Default)]
struct StateMachine {
    users: HashMap<String, HashMap<String, TaskMeta>>, // user_id -> (node_id -> meta)
    history: HashMap<String, VecDeque<SmEventDigest>>, // user_id -> recent applied event digests
}

#[derive(Debug, Clone)]
enum SmEffect {
    SetState {
        user_id: String,
        node_id: String,
        state: TaskState,
    },
    EnqueueReady {
        user_id: String,
        node_id: String,
        priority: i32,
    },
}

#[derive(Debug, Clone)]
enum SmEvent {
    /// 初始化/重置某个 user 的 workflow 实例：Created 全量投影 + start 节点入队 Ready。
    UserReset { user_id: String },

    /// 调度器从 ready queue 取出一个 task，准备派发执行（Ready -> Running）。
    DispatchStart { user_id: String, node_id: String },

    /// 收到 packet.rx 事件后，尝试触发 wait 节点（Waiting -> Completed + edge advance）。
    PacketRx {
        user_id: String,
        action_id: String,
        task_id: String,
        payload: PacketRxPayload,
        eval_ctx: serde_json::Value,
    },

    /// 收到 scheduler.action-result 事件后，更新节点状态并按需要推进边。
    ActionResult {
        user_id: String,
        node_id: String,
        reason: String, // success/failed/timeout
        success: bool,
        should_advance: bool,
        /// success 且 node 还有下一步 action：不沿边推进，而是将自身置回 Ready 并重新入队
        continue_node: bool,
        eval_ctx: serde_json::Value,
    },

    /// 重试 timer 到期（Failed -> Ready 入队）。
    RetryTimer { user_id: String, node_id: String },

    /// timeout timer 到期：Running -> Failed（后续由 action-result(timeout) 再推进）。
    TimeoutTimer { user_id: String, node_id: String },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SmEventDigest {
    ts_ms: u64,
    kind: String,
    node_id: Option<String>,
    reason: Option<String>,
}

impl StateMachine {
    fn ensure_user(&mut self, user_id: &str) {
        self.users.entry(user_id.to_string()).or_default();
        self.history.entry(user_id.to_string()).or_default();
    }

    fn get_state(&self, user_id: &str, node_id: &str) -> Option<TaskState> {
        self.users
            .get(user_id)
            .and_then(|m| m.get(node_id))
            .map(|m| m.state)
    }

    fn get_step(&self, user_id: &str, node_id: &str) -> u32 {
        self.users
            .get(user_id)
            .and_then(|m| m.get(node_id))
            .map(|m| m.step_idx)
            .unwrap_or(0)
    }

    fn set_step(&mut self, user_id: &str, node_id: &str, step_idx: u32, now_ms: u64) {
        self.ensure_user(user_id);
        let m = self.users.get_mut(user_id).unwrap();
        m.entry(node_id.to_string())
            .and_modify(|x| {
                x.step_idx = step_idx;
                x.last_update_ms = now_ms;
            })
            .or_insert(TaskMeta {
                state: TaskState::Created,
                last_update_ms: now_ms,
                step_idx,
            });
    }

    fn set_state(&mut self, user_id: &str, node_id: &str, state: TaskState, now_ms: u64) {
        self.ensure_user(user_id);
        let m = self.users.get_mut(user_id).unwrap();
        m.entry(node_id.to_string())
            .and_modify(|x| {
                x.state = state;
                x.last_update_ms = now_ms;
            })
            .or_insert(TaskMeta {
                state,
                last_update_ms: now_ms,
                step_idx: 0,
            });
    }

    fn record_event(&mut self, user_id: &str, d: SmEventDigest) {
        self.ensure_user(user_id);
        let h = self.history.get_mut(user_id).unwrap();
        h.push_back(d);
        while h.len() > 256 {
            h.pop_front();
        }
    }

    fn digest(now_ms: u64, ev: &SmEvent) -> SmEventDigest {
        match ev {
            SmEvent::UserReset { .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: "user.reset".to_string(),
                node_id: None,
                reason: None,
            },
            SmEvent::DispatchStart { node_id, .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: "dispatch.start".to_string(),
                node_id: Some(node_id.clone()),
                reason: None,
            },
            SmEvent::PacketRx { .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: "packet.rx".to_string(),
                node_id: None,
                reason: None,
            },
            SmEvent::ActionResult {
                node_id, reason, ..
            } => SmEventDigest {
                ts_ms: now_ms,
                kind: "action.result".to_string(),
                node_id: Some(node_id.clone()),
                reason: Some(reason.clone()),
            },
            SmEvent::RetryTimer { node_id, .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: "timer.retry".to_string(),
                node_id: Some(node_id.clone()),
                reason: None,
            },
            SmEvent::TimeoutTimer { node_id, .. } => SmEventDigest {
                ts_ms: now_ms,
                kind: "timer.timeout".to_string(),
                node_id: Some(node_id.clone()),
                reason: None,
            },
        }
    }

    /// 唯一入口：对状态机应用一个事件，返回需要落地到 runtime/ready-queue 的 effects。
    fn apply(
        &mut self,
        sc: &Scenario,
        wf_index: &WorkflowIndex,
        now_ms: u64,
        ev: SmEvent,
    ) -> Vec<SmEffect> {
        let uid = match &ev {
            SmEvent::UserReset { user_id }
            | SmEvent::DispatchStart { user_id, .. }
            | SmEvent::PacketRx { user_id, .. }
            | SmEvent::ActionResult { user_id, .. }
            | SmEvent::RetryTimer { user_id, .. }
            | SmEvent::TimeoutTimer { user_id, .. } => user_id.as_str(),
        };
        self.record_event(uid, Self::digest(now_ms, &ev));

        match ev {
            SmEvent::UserReset { user_id } => self.reset_user(sc, &user_id, now_ms),
            SmEvent::DispatchStart { user_id, node_id } => {
                if self.get_state(&user_id, &node_id) != Some(TaskState::Ready) {
                    return Vec::new();
                }
                self.set_state(&user_id, &node_id, TaskState::Running, now_ms);
                vec![SmEffect::SetState {
                    user_id,
                    node_id,
                    state: TaskState::Running,
                }]
            }
            SmEvent::PacketRx {
                user_id,
                action_id,
                task_id,
                payload,
                eval_ctx,
            } => self.on_packet_rx(
                sc, wf_index, &user_id, &action_id, &task_id, &payload, &eval_ctx, now_ms,
            ),
            SmEvent::ActionResult {
                user_id,
                node_id,
                reason,
                success,
                should_advance,
                continue_node,
                eval_ctx,
            } => self.on_action_result(
                sc,
                &user_id,
                &node_id,
                &reason,
                &eval_ctx,
                should_advance,
                continue_node,
                now_ms,
                success,
            ),
            SmEvent::RetryTimer { user_id, node_id } => {
                self.on_retry_timer(sc, &user_id, &node_id, now_ms)
            }
            SmEvent::TimeoutTimer { user_id, node_id } => {
                if self.get_state(&user_id, &node_id) != Some(TaskState::Running) {
                    return Vec::new();
                }
                self.set_state(&user_id, &node_id, TaskState::Failed, now_ms);
                vec![SmEffect::SetState {
                    user_id,
                    node_id,
                    state: TaskState::Failed,
                }]
            }
        }
    }

    fn reset_user(&mut self, sc: &Scenario, user_id: &str, now_ms: u64) -> Vec<SmEffect> {
        self.ensure_user(user_id);
        let mut eff = Vec::new();
        // all nodes Created
        for n in &sc.workflows.nodes {
            self.set_state(user_id, &n.id, TaskState::Created, now_ms);
            self.set_step(user_id, &n.id, 0, now_ms);
            eff.push(SmEffect::SetState {
                user_id: user_id.to_string(),
                node_id: n.id.clone(),
                state: TaskState::Created,
            });
        }
        // start nodes Ready
        for nid in find_start_nodes(sc) {
            self.set_state(user_id, &nid, TaskState::Ready, now_ms);
            eff.push(SmEffect::SetState {
                user_id: user_id.to_string(),
                node_id: nid.clone(),
                state: TaskState::Ready,
            });
            eff.push(SmEffect::EnqueueReady {
                user_id: user_id.to_string(),
                node_id: nid.clone(),
                priority: node_priority(sc, &nid),
            });
        }
        eff
    }

    fn on_retry_timer(
        &mut self,
        sc: &Scenario,
        user_id: &str,
        node_id: &str,
        now_ms: u64,
    ) -> Vec<SmEffect> {
        // only Failed -> Ready
        if self.get_state(user_id, node_id) != Some(TaskState::Failed) {
            return Vec::new();
        }
        self.set_state(user_id, node_id, TaskState::Ready, now_ms);
        vec![
            SmEffect::SetState {
                user_id: user_id.to_string(),
                node_id: node_id.to_string(),
                state: TaskState::Ready,
            },
            SmEffect::EnqueueReady {
                user_id: user_id.to_string(),
                node_id: node_id.to_string(),
                priority: node_priority(sc, node_id),
            },
        ]
    }

    fn on_packet_rx(
        &mut self,
        sc: &Scenario,
        wf_index: &WorkflowIndex,
        user_id: &str,
        action_id: &str,
        task_id: &str,
        p: &PacketRxPayload,
        eval_ctx: &serde_json::Value,
        now_ms: u64,
    ) -> Vec<SmEffect> {
        // 候选 wait 节点：优先按 match.action_id 命中索引，否则走 wait_any
        let mut candidates: Vec<String> = Vec::new();
        candidates.extend(wf_index.wait_any.iter().cloned());
        if !action_id.is_empty() {
            if let Some(v) = wf_index.wait_by_action_id.get(action_id) {
                candidates.extend(v.iter().cloned());
            }
        }

        let wait_nodes: Vec<String> = candidates
            .into_iter()
            .filter(|nid| {
                sc.workflows
                    .nodes
                    .iter()
                    .find(|n| &n.id == nid)
                    .map(|n| {
                        n.kind == "wait"
                            && n.on
                                .as_ref()
                                .map(|o| o.event.as_str() == "packet.rx")
                                .unwrap_or(false)
                            && wait_match(n.on.as_ref(), action_id, task_id, p)
                    })
                    .unwrap_or(false)
            })
            .collect();

        if wait_nodes.is_empty() {
            return Vec::new();
        }

        let mut eff = Vec::new();
        for wait_id in wait_nodes {
            if self.get_state(user_id, &wait_id) != Some(TaskState::Waiting) {
                continue;
            }
            self.set_state(user_id, &wait_id, TaskState::Completed, now_ms);
            eff.push(SmEffect::SetState {
                user_id: user_id.to_string(),
                node_id: wait_id.clone(),
                state: TaskState::Completed,
            });
            eff.extend(self.advance_edges(
                sc,
                user_id,
                &wait_id,
                "packet.rx",
                Some(eval_ctx),
                now_ms,
            ));
        }
        eff
    }

    fn on_action_result(
        &mut self,
        sc: &Scenario,
        user_id: &str,
        node_id: &str,
        reason: &str,
        eval_ctx: &serde_json::Value,
        should_advance: bool,
        continue_node: bool,
        now_ms: u64,
        success: bool,
    ) -> Vec<SmEffect> {
        // success + continue_node: Ready + re-enqueue same node
        if success && continue_node {
            self.set_state(user_id, node_id, TaskState::Ready, now_ms);
            return vec![
                SmEffect::SetState {
                    user_id: user_id.to_string(),
                    node_id: node_id.to_string(),
                    state: TaskState::Ready,
                },
                SmEffect::EnqueueReady {
                    user_id: user_id.to_string(),
                    node_id: node_id.to_string(),
                    priority: node_priority(sc, node_id),
                },
            ];
        }

        // Running -> Completed/Failed (best-effort; allow idempotent replays)
        let st = if success {
            TaskState::Completed
        } else {
            TaskState::Failed
        };
        self.set_state(user_id, node_id, st, now_ms);

        let mut eff = vec![SmEffect::SetState {
            user_id: user_id.to_string(),
            node_id: node_id.to_string(),
            state: st,
        }];

        if should_advance {
            eff.extend(self.advance_edges(sc, user_id, node_id, reason, Some(eval_ctx), now_ms));
        }
        eff
    }

    fn is_end_reached(&self, sc: &Scenario, user_id: &str) -> bool {
        let end_nodes: Vec<&WorkflowNodeDef> = sc
            .workflows
            .nodes
            .iter()
            .filter(|n| n.kind == "end")
            .collect();
        if end_nodes.is_empty() {
            return false;
        }
        end_nodes
            .into_iter()
            .any(|n| self.get_state(user_id, &n.id) == Some(TaskState::Completed))
    }

    fn advance_edges(
        &mut self,
        sc: &Scenario,
        user_id: &str,
        from_node_id: &str,
        reason: &str,
        eval_ctx: Option<&serde_json::Value>,
        now_ms: u64,
    ) -> Vec<SmEffect> {
        let Some(from) = sc.workflows.nodes.iter().find(|n| n.id == from_node_id) else {
            return Vec::new();
        };

        let mut eff = Vec::new();
        for e in &from.edges {
            if !edge_trigger_allows(e.trigger.as_ref(), reason, eval_ctx) {
                continue;
            }
            let Some(to) = sc.workflows.nodes.iter().find(|n| n.id == e.to) else {
                continue;
            };
            match to.kind.as_str() {
                "wait" => {
                    self.set_state(user_id, &to.id, TaskState::Waiting, now_ms);
                    eff.push(SmEffect::SetState {
                        user_id: user_id.to_string(),
                        node_id: to.id.clone(),
                        state: TaskState::Waiting,
                    });
                }
                "action" => {
                    self.set_state(user_id, &to.id, TaskState::Ready, now_ms);
                    eff.push(SmEffect::SetState {
                        user_id: user_id.to_string(),
                        node_id: to.id.clone(),
                        state: TaskState::Ready,
                    });
                    eff.push(SmEffect::EnqueueReady {
                        user_id: user_id.to_string(),
                        node_id: to.id.clone(),
                        priority: node_priority(sc, &to.id),
                    });
                }
                "end" => {
                    self.set_state(user_id, &to.id, TaskState::Completed, now_ms);
                    eff.push(SmEffect::SetState {
                        user_id: user_id.to_string(),
                        node_id: to.id.clone(),
                        state: TaskState::Completed,
                    });
                }
                _ => {}
            }
        }
        eff
    }
}

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
        publish_user_start_event(1, None);
    } else {
        // 允许 phase.at_second == 0 立即触发
        tick_load_controller();
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
    max_ticks: u32,
) -> Result<(), String> {
    let mut idle = 0u32;
    for _ in 0..max_ticks {
        let mut did_work = false;

        // 1) control events
        if let Some(id) = sub_ctrl {
            let events = ntx::scenario_eventbus::event_bus::poll_events(id, 64)
                .map_err(|e| format!("poll_events(control): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                on_control_event(&ev);
            }
        }

        // stop flag
        if RUNTIME.lock().map(|rt| rt.stop).unwrap_or(false) {
            break;
        }

        if let Some(id) = sub_tx {
            let events = ntx::scenario_eventbus::event_bus::poll_events(id, 64)
                .map_err(|e| format!("poll_events(packet.tx-request): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "packet.tx-request" {
                    if let Err(e) = handle_tx_request(&ev.payload, ev.correlation_id.as_deref()) {
                        println!("[scheduler] process tx-request failed: {e}");
                    }
                }
            }
        }

        // 1.5) send schedule requests (executor -> scheduler)
        if let Some(id) = sub_send {
            let events = ntx::scenario_eventbus::event_bus::poll_events(id, 64)
                .map_err(|e| format!("poll_events(send.schedule-request): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "send.schedule-request" {
                    if let Err(e) = on_send_schedule_request(&ev) {
                        println!("[scheduler] handle send.schedule-request failed: {e}");
                    }
                }
            }
        }

        // 2) action-result events (async-compatible)
        if let Some(id) = sub_ar {
            let events = ntx::scenario_eventbus::event_bus::poll_events(id, 64)
                .map_err(|e| format!("poll_events(action-result): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "scheduler.action-result" {
                    on_action_result_event(ctx, &ev)?;
                }
            }
        }

        if let Some(id) = sub_rx {
            let events = ntx::scenario_eventbus::event_bus::poll_events(id, 64)
                .map_err(|e| format!("poll_events(packet.rx): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "packet.rx" {
                    on_packet_rx(ctx, &ev)?;
                }
            }
        }

        // 3) timer events
        if let Some(id) = sub_timer {
            let events = ntx::scenario_eventbus::event_bus::poll_events(id, 64)
                .map_err(|e| format!("poll_events(timer): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind.starts_with("scheduler.timer.") {
                    on_timer_event(ctx, &ev)?;
                }
            }
        }

        // 4) user lifecycle events
        if let Some(id) = sub_user {
            let events = ntx::scenario_eventbus::event_bus::poll_events(id, 64)
                .map_err(|e| format!("poll_events(user): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                match ev.kind.as_str() {
                    "scheduler.user.start" => on_user_start_event(ctx, &ev)?,
                    "scheduler.user.exit" => on_user_exit_event(ctx, &ev)?,
                    _ => {}
                }
            }
        }

        // 4.1) topology change events (affect only NEW users)
        if let Some(id) = sub_topo {
            let events = ntx::scenario_eventbus::event_bus::poll_events(id, 16)
                .map_err(|e| format!("poll_events(topology): {e}"))?;
            if !events.is_empty() {
                did_work = true;
            }
            for ev in events {
                if ev.kind == "topology.changed" {
                    if let Err(e) = on_topology_changed_event(ctx, &ev) {
                        // on_topology_changed_event already published scheduler.topology.rejected with rich payload
                        println!("[scheduler] warn: topology.changed rejected: {}", e);
                    }
                }
            }
        }

        // 5) drive load/timers
        tick_load_controller();
        tick_timers();

        let paused = RUNTIME.lock().map(|rt| rt.paused).unwrap_or(false);
        if !paused {
            // 调度 ready tasks（最小实现：每 tick 尝试跑一批）
            did_work |= dispatch_ready_tasks(ctx, 16)?;
        }

        // send-scheduler tick：检查到期的 send-request 并发包
        tick_send_scheduler(now_ms());
        // best-effort cleanup
        cleanup_sock_ctx(now_ms(), 60_000);

        if did_work {
            idle = 0;
        } else {
            idle += 1;
        }

        if idle >= 10 {
            let has_active_send = SEND_JOBS
                .lock()
                .map(|m| m.values().any(is_job_active))
                .unwrap_or(false);
            let has_ready = RUNTIME
                .lock()
                .map(|rt| !rt.ready.is_empty())
                .unwrap_or(false);
            if !has_active_send && !has_ready {
                break;
            }
        }
    }
    Ok(())
}

fn on_packet_rx(
    ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    handle_packet_rx_trigger(ctx, ev)
}

fn handle_packet_rx_trigger(
    ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let Some(ctx_user) = ev.user_id.as_deref() else {
        return Ok(());
    };
    let action_id = ev.action_id.as_deref().unwrap_or("");
    let task_id = ev.task_id.as_deref().unwrap_or("");
    let (_ver, sc_arc, wf_idx) = get_user_scenario_ctx(ctx_user)?;
    let sc = sc_arc.as_ref();

    let p: PacketRxPayload = serde_json::from_str(&ev.payload).unwrap_or_default();
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
            now_ms(),
            SmEvent::PacketRx {
                user_id: ctx_user.to_string(),
                action_id: action_id.to_string(),
                task_id: task_id.to_string(),
                payload: p,
                eval_ctx,
            },
        )
    };
    apply_sm_effects(effects)?;
    maybe_finish_user(ctx, ctx_user)?;
    Ok(())
}

fn wait_match(
    on: Option<&WaitOnSpec>,
    action_id: &str,
    task_id: &str,
    p: &PacketRxPayload,
) -> bool {
    let Some(on) = on else {
        return false;
    };
    let m = &on.r#match;
    if m.is_null() {
        return true;
    }
    // 支持 match.action_id / match.task_id
    let mut ok = true;
    if let Some(exp) = m.get("action_id").and_then(|v| v.as_str()) {
        ok &= exp == action_id;
    }
    if let Some(exp) = m.get("task_id").and_then(|v| v.as_str()) {
        ok &= exp == task_id;
    }
    // 支持 match.sock_id / match.len / match.payload_hex
    if let Some(exp) = m
        .get("sock_id")
        .or_else(|| m.get("sock-id"))
        .and_then(|v| v.as_u64())
    {
        ok &= p.sock_id == exp;
    }
    if let Some(exp) = m.get("len").and_then(|v| v.as_u64()) {
        ok &= u64::try_from(p.len)
            .ok()
            .map(|got| got == exp)
            .unwrap_or(false);
    }
    if let Some(exp) = m
        .get("payload_hex")
        .or_else(|| m.get("payload-hex"))
        .and_then(|v| v.as_str())
    {
        let norm = |s: &str| s.trim().trim_start_matches("0x").to_ascii_lowercase();
        ok &= norm(&p.payload_hex) == norm(exp);
    }
    ok
}

fn on_control_event(ev: &ntx::scenario_eventbus::event_bus::Event) {
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

fn on_timer_event(
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

fn on_think_timer(
    ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(&ev.payload).unwrap_or(serde_json::json!({}));
    let user_id = v.get("user_id").and_then(|x| x.as_str()).unwrap_or("");
    if user_id.is_empty() {
        return Ok(());
    }
    restart_user_iteration(ctx, user_id)
}

fn on_timeout_timer(
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

    // only apply if still Running (authoritative: StateMachine) via apply(TimeoutTimer)
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
        let (_ver, sc_arc, wf_idx) = get_user_scenario_ctx(user_id)?;
        let sc = sc_arc.as_ref();
        let mut sm = STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        let effects = sm.apply(
            sc,
            &wf_idx,
            now_ms(),
            SmEvent::TimeoutTimer {
                user_id: user_id.to_string(),
                node_id: task_id.to_string(),
            },
        );
        let did = !effects.is_empty();
        if did {
            // keep runtime derived state in sync
            apply_sm_effects(effects)?;
        }
        did
    };
    if should_timeout {
        // derive state to runtime + decrement running if needed
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
                timestamp_ms: now_ms(),
            });
    }
    Ok(())
}

fn on_retry_timer(
    _ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(&ev.payload).unwrap_or(serde_json::json!({}));
    let user_id = v.get("user_id").and_then(|x| x.as_str()).unwrap_or("");
    let task_id = v.get("task_id").and_then(|x| x.as_str()).unwrap_or("");
    if user_id.is_empty() || task_id.is_empty() {
        return Ok(());
    }

    let (_ver, sc_arc, wf_idx) = get_user_scenario_ctx(user_id)?;
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
            now_ms(),
            SmEvent::RetryTimer {
                user_id: user_id.to_string(),
                node_id: task_id.to_string(),
            },
        )
    };
    apply_sm_effects(effects)?;
    Ok(())
}

fn node_priority(sc: &Scenario, node_id: &str) -> i32 {
    sc.workflows
        .nodes
        .iter()
        .find(|n| n.id == node_id)
        .and_then(|n| n.priority)
        .unwrap_or(0)
}

fn edge_trigger_allows(
    trigger: Option<&serde_json::Value>,
    reason: &str,
    eval_ctx: Option<&serde_json::Value>,
) -> bool {
    let Some(t) = trigger else {
        return true;
    };
    if t.is_null() {
        return true;
    }
    let Some(obj) = t.as_object() else {
        return true;
    };

    // 最小支持：trigger.on / trigger.event / trigger.status
    let mut allow = None::<bool>;
    if let Some(on) = obj.get("on").and_then(|v| v.as_str()) {
        allow = Some(match_reason(on, reason));
    }
    if allow.is_none() {
        if let Some(ev) = obj.get("event").and_then(|v| v.as_str()) {
            allow = Some(match_reason(ev, reason));
        }
    }
    if allow.is_none() {
        if let Some(st) = obj.get("status").and_then(|v| v.as_str()) {
            allow = Some(match_reason(st, reason));
        }
    }

    // 先按 on/event/status 过滤：若显式指定且不匹配，则拒绝
    if matches!(allow, Some(false)) {
        return false;
    }

    // trigger.condition：受限表达式（==/!=/contains + &&/||），基于 eval_ctx 求值
    if let Some(cond) = obj.get("condition").and_then(|v| v.as_str()) {
        let Some(ec) = eval_ctx else {
            println!(
                "[scheduler] warn: edge trigger.condition provided but no eval_ctx, rejecting: {}",
                t
            );
            return false;
        };
        match eval_condition(cond, ec) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => {
                println!(
                    "[scheduler] warn: eval condition failed, rejecting edge; condition=`{}` err={}",
                    cond, e
                );
                return false;
            }
        }
    }

    // allow==Some(true) or allow==None(default allow)
    allow.unwrap_or(true)
}

fn match_reason(expr: &str, reason: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .to_ascii_lowercase()
            .replace('_', ".")
            .replace('-', ".")
    };
    let a = norm(expr);
    let b = norm(reason);
    // allow common aliases
    match (a.as_str(), b.as_str()) {
        ("packet.rx", "packet.rx") => true,
        ("success", "success") => true,
        ("failed", "failed") => true,
        ("failure", "failed") => true,
        ("timeout", "timeout") => true,
        ("action.success", "success") => true,
        ("action.failed", "failed") => true,
        ("action.timeout", "timeout") => true,
        _ => a == b,
    }
}

fn eval_condition(expr: &str, ctx: &serde_json::Value) -> Result<bool, String> {
    // OR has lower precedence than AND
    let ors = split_outside_quotes(expr, "||");
    if ors.len() > 1 {
        for part in ors {
            if eval_condition(part.trim(), ctx)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    let ands = split_outside_quotes(expr, "&&");
    if ands.len() > 1 {
        for part in ands {
            if !eval_condition(part.trim(), ctx)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    eval_atom(expr.trim(), ctx)
}

fn eval_atom(expr: &str, ctx: &serde_json::Value) -> Result<bool, String> {
    // Support: <path> == <lit> | != | contains
    if expr.is_empty() {
        return Ok(true);
    }
    // contains
    if let Some((lhs, rhs)) = split_once_outside_quotes(expr, "contains") {
        let lhs = lhs.trim();
        let rhs = rhs.trim();
        let lv = resolve_path(ctx, lhs);
        let rv = parse_literal(rhs)?;
        let ls = value_to_string(lv);
        let rs = value_to_string(&rv);
        return Ok(ls.contains(&rs));
    }
    // ==
    if let Some((lhs, rhs)) = split_once_outside_quotes(expr, "==") {
        let lv = resolve_path(ctx, lhs.trim());
        let rv = parse_literal(rhs.trim())?;
        return Ok(values_equal(lv, &rv));
    }
    // !=
    if let Some((lhs, rhs)) = split_once_outside_quotes(expr, "!=") {
        let lv = resolve_path(ctx, lhs.trim());
        let rv = parse_literal(rhs.trim())?;
        return Ok(!values_equal(lv, &rv));
    }
    // Bare identifier: treat as truthy (exists && not false/null/0/"")
    let v = resolve_path(ctx, expr);
    Ok(is_truthy(v))
}

fn is_truthy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_i64().map(|x| x != 0).unwrap_or(true),
        serde_json::Value::String(s) => {
            !s.is_empty() && s != "0" && s.to_ascii_lowercase() != "false"
        }
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
    }
}

fn values_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    // Prefer numeric compare when both numeric
    if let (Some(na), Some(nb)) = (a.as_f64(), b.as_f64()) {
        return (na - nb).abs() < f64::EPSILON;
    }
    // Else direct JSON equality (covers bool/null/object/array) OR string normalized
    if a == b {
        return true;
    }
    value_to_string(a) == value_to_string(b)
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn parse_literal(s: &str) -> Result<serde_json::Value, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(serde_json::Value::String(String::new()));
    }
    // quoted string
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        let inner = &s[1..s.len().saturating_sub(1)];
        return Ok(serde_json::Value::String(inner.to_string()));
    }
    // bool/null
    let lc = s.to_ascii_lowercase();
    if lc == "true" {
        return Ok(serde_json::Value::Bool(true));
    }
    if lc == "false" {
        return Ok(serde_json::Value::Bool(false));
    }
    if lc == "null" {
        return Ok(serde_json::Value::Null);
    }
    // number
    if let Ok(i) = s.parse::<i64>() {
        return Ok(serde_json::Value::Number(i.into()));
    }
    if let Ok(f) = s.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Ok(serde_json::Value::Number(n));
        }
    }
    // fallback: bareword string
    Ok(serde_json::Value::String(s.to_string()))
}

fn resolve_path<'a>(ctx: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    static NULL: serde_json::Value = serde_json::Value::Null;
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty()).collect();
    let mut cur = ctx;
    for p in parts {
        match cur.get(p) {
            Some(v) => cur = v,
            None => return &NULL,
        }
    }
    cur
}

fn split_once_outside_quotes<'a>(s: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    // For "contains" we expect it to be delimited by whitespace or operators; but keep MVP.
    let mut in_sq = false;
    let mut in_dq = false;
    let bytes = s.as_bytes();
    let opb = op.as_bytes();
    let mut i = 0usize;
    while i + opb.len() <= bytes.len() {
        let c = bytes[i] as char;
        if c == '\'' && !in_dq {
            in_sq = !in_sq;
            i += 1;
            continue;
        }
        if c == '"' && !in_sq {
            in_dq = !in_dq;
            i += 1;
            continue;
        }
        if !in_sq && !in_dq && &bytes[i..i + opb.len()] == opb {
            let (a, b) = s.split_at(i);
            let b = &b[op.len()..];
            return Some((a, b));
        }
        i += 1;
    }
    None
}

fn split_outside_quotes<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut in_sq = false;
    let mut in_dq = false;
    let bytes = s.as_bytes();
    let sepb = sep.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + sepb.len() <= bytes.len() {
        let c = bytes[i] as char;
        if c == '\'' && !in_dq {
            in_sq = !in_sq;
            i += 1;
            continue;
        }
        if c == '"' && !in_sq {
            in_dq = !in_dq;
            i += 1;
            continue;
        }
        if !in_sq && !in_dq && &bytes[i..i + sepb.len()] == sepb {
            out.push(&s[start..i]);
            i += sepb.len();
            start = i;
            continue;
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

fn build_workflow_index(sc: &Scenario) -> WorkflowIndex {
    let mut idx = WorkflowIndex::default();
    for n in &sc.workflows.nodes {
        if n.kind != "wait" {
            continue;
        }
        let Some(on) = n.on.as_ref() else {
            idx.wait_any.push(n.id.clone());
            continue;
        };
        if on.event.as_str() != "packet.rx" {
            idx.wait_any.push(n.id.clone());
            continue;
        }
        let m = &on.r#match;
        let action_id = m
            .get("action_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let Some(aid) = action_id {
            idx.wait_by_action_id
                .entry(aid)
                .or_default()
                .push(n.id.clone());
        } else {
            idx.wait_any.push(n.id.clone());
        }
    }
    idx
}

fn get_active_scenario_ctx() -> Result<(u64, Arc<Scenario>, WorkflowIndex), String> {
    let reg = SCENARIOS
        .lock()
        .map_err(|_| "lock scenario registry".to_string())?;
    reg.active().ok_or_else(|| "no active scenario".to_string())
}

fn get_user_scenario_ctx(user_id: &str) -> Result<(u64, Arc<Scenario>, WorkflowIndex), String> {
    let ver = {
        let rt = RUNTIME.lock().map_err(|_| "lock runtime".to_string())?;
        rt.users
            .get(user_id)
            .map(|u| u.meta.scenario_version)
            .unwrap_or(1)
    };
    let reg = SCENARIOS
        .lock()
        .map_err(|_| "lock scenario registry".to_string())?;
    let (sc, idx) = reg
        .by_version(ver)
        .ok_or_else(|| format!("scenario version not found: {}", ver))?;
    Ok((ver, sc, idx))
}

fn on_topology_changed_event(
    _ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    // Normalized protocol (JSON, strict):
    // {
    //   "schema_version": 1,
    //   "change_id": "uuid-or-human",
    //   "mode": "replace-yaml" | "replace-json" | "patch",
    //   "base_version": 3,              // optional; required for patch for optimistic concurrency
    //   ... mode-specific fields ...
    // }
    #[derive(Debug, Clone, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TopologyChangedEnvelope {
        schema_version: u32,
        change_id: String,
        #[serde(default)]
        base_version: Option<u64>,
        #[serde(flatten)]
        change: TopologyChangedBody,
    }

    #[derive(Debug, Clone, Deserialize)]
    #[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
    enum TopologyChangedBody {
        ReplaceYaml { scenario_yaml: String },
        ReplaceJson { scenario_json: serde_json::Value },
        Patch { ops: Vec<PatchOp> },
    }

    #[derive(Debug, Clone, Deserialize)]
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
            node: WorkflowNodeDef,
        },
        UpsertAction {
            action: Action,
        },
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
                    // also remove incoming edges to this node (best-effort)
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
    // resolve base_version and build new scenario
    let (base_ver, base_sc) = {
        let reg = SCENARIOS
            .lock()
            .map_err(|_| "lock scenario registry".to_string())?;
        let (active_ver, sc, _idx) = reg
            .active()
            .ok_or_else(|| "no active scenario".to_string())?;
        let want = env.base_version.unwrap_or(active_ver);
        if want != active_ver {
            // strict: only allow patching from current active_version to avoid ambiguity
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

    // update load controller phases for future spawns (existing users do not migrate)
    if let Ok(mut lc) = LOAD.lock() {
        if let Ok(reg) = SCENARIOS.lock() {
            if let Some((sc, _idx)) = reg.by_version(new_ver) {
                lc.phases = sc.load.ramp_up.phases.clone();
                lc.next_phase = 0;
                // keep next_user_seq so user ids remain monotonic
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

fn on_user_start_event(
    _ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let (ver, sc_arc, wf_idx) = get_active_scenario_ctx()?;
    let sc = sc_arc.as_ref();
    let v: serde_json::Value = serde_json::from_str(&ev.payload).unwrap_or(serde_json::json!({}));
    let user_id = v.get("user_id").and_then(|x| x.as_str()).unwrap_or("");
    if user_id.is_empty() {
        return Ok(());
    }

    let resources = build_resources_json(sc);
    let mut user = UserInstance {
        tasks: HashMap::new(),
        resources,
        meta: {
            let mut m = user_meta_from_config(&sc.load.user_lifetime);
            m.scenario_version = ver;
            m
        },
    };
    for n in &sc.workflows.nodes {
        let tr = TaskRuntime {
            state: TaskState::Created,
            vars: serde_json::json!({}),
            exports: serde_json::json!({}),
        };
        user.tasks.insert(n.id.clone(), tr);
    }

    if let Ok(mut rt) = RUNTIME.lock() {
        if rt.users.contains_key(user_id) {
            return Ok(());
        }
        rt.users.insert(user_id.to_string(), user);
    }
    // authoritative init via StateMachine
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

fn on_user_exit_event(
    _ctx: &SchedulerContext,
    ev: &ntx::scenario_eventbus::event_bus::Event,
) -> Result<(), String> {
    let Some(user_id) = ev.user_id.as_deref() else {
        return Ok(());
    };
    let (_ver, sc_arc, _wf_idx) = get_user_scenario_ctx(user_id)?;
    let sc = sc_arc.as_ref();

    // read current meta
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
        return finish_user(_ctx, user_id);
    }

    // loop mode: increment iteration and decide stop/restart
    let next_iter = cur_iter.saturating_add(1);
    let should_stop = iterations.map(|n| next_iter >= n).unwrap_or(false);

    if let Ok(mut rt) = RUNTIME.lock() {
        if let Some(u) = rt.users.get_mut(user_id) {
            u.meta.iteration = next_iter;
            u.meta.end_event_sent = false;
        }
    }

    if should_stop {
        return finish_user(_ctx, user_id);
    }

    if let Some(ms) = think_ms {
        schedule_timer(
            "scheduler.timer.think",
            now_ms().saturating_add(ms),
            user_id,
            "user",
            None,
            serde_json::json!({"user_id": user_id, "iteration": next_iter}),
        );
        Ok(())
    } else {
        restart_user_iteration_with_scenario(sc, user_id)
    }
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

fn tick_load_controller() {
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
    let now = now_ms();
    let elapsed_sec = now.saturating_sub(started_at_ms) / 1000;
    let mut idx = next_phase;
    let mut seq = next_user_seq;

    while idx < phases.len() && phases[idx].at_second <= elapsed_sec {
        let spawn = phases[idx].spawn_users;
        publish_user_start_event(spawn as u64, Some(seq));
        seq += spawn as u64;
        idx += 1;
    }

    if let Ok(mut lc) = LOAD.lock() {
        lc.next_phase = idx;
        lc.next_user_seq = seq;
    }
}

fn find_start_nodes(sc: &Scenario) -> Vec<String> {
    if sc.workflows.nodes.iter().any(|n| n.id == "start") {
        return vec!["start".to_string()];
    }
    // no incoming edges nodes
    let mut has_incoming: HashMap<&str, bool> = HashMap::new();
    for n in &sc.workflows.nodes {
        has_incoming.insert(&n.id, false);
    }
    for n in &sc.workflows.nodes {
        for e in &n.edges {
            has_incoming.insert(&e.to, true);
        }
    }
    sc.workflows
        .nodes
        .iter()
        .filter(|n| !has_incoming.get(n.id.as_str()).copied().unwrap_or(false))
        .map(|n| n.id.clone())
        .collect()
}

fn publish_user_start_event(spawn_users: u64, start_seq: Option<u64>) {
    let base = start_seq.unwrap_or(1);
    for i in 0..spawn_users {
        let user_id = format!("user-{}", base + i);
        let payload = serde_json::json!({ "user_id": user_id }).to_string();
        let id = format!("us-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed));
        let _ =
            ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
                id,
                kind: "scheduler.user.start".to_string(),
                user_id: None,
                task_id: None,
                action_id: None,
                payload,
                correlation_id: None,
                timestamp_ms: now_ms(),
            });
    }
}

fn tick_timers() {
    let now = now_ms();
    let mut due: Vec<TimerJob> = Vec::new();
    if let Ok(mut timers) = TIMERS.lock() {
        let mut i = 0usize;
        while i < timers.len() {
            if timers[i].due_ms <= now {
                due.push(timers.remove(i));
            } else {
                i += 1;
            }
        }
    }

    for t in due {
        let id = format!("tm-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed));
        let _ =
            ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
                id,
                kind: t.kind,
                user_id: t.user_id,
                task_id: t.task_id,
                action_id: t.action_id,
                payload: t.payload,
                correlation_id: None,
                timestamp_ms: now,
            });
    }
}

fn schedule_timer(
    kind: &str,
    due_ms: u64,
    user_id: &str,
    task_id: &str,
    action_id: Option<&str>,
    payload: serde_json::Value,
) {
    let job = TimerJob {
        due_ms,
        kind: kind.to_string(),
        user_id: Some(user_id.to_string()),
        task_id: Some(task_id.to_string()),
        action_id: action_id.map(|s| s.to_string()),
        payload: payload.to_string(),
    };
    if let Ok(mut timers) = TIMERS.lock() {
        timers.push(job);
    }
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

        let action = match sc.actions.actions.iter().find(|a| a.id == action_id) {
            Some(a) => a,
            None => continue,
        };

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

        let outcome =
            ntx::scenario_actions_executor::action_component::execute_action(&def, Some(&act_ctx))
                .map_err(|e| format!("execute_action failed: {e}"))?;

        // 结果事件化：发布 action-result，由事件处理器更新状态/重试/超时（兼容 future async）
        publish_action_result_event(
            &user_id,
            &node_id,
            &def.id,
            act_ctx.correlation_id.as_deref(),
            &outcome,
        )?;
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
    let sock_id = {
        let rt = RUNTIME.lock().map_err(|_| "lock runtime".to_string())?;
        let u = rt
            .users
            .get(user_id)
            .ok_or_else(|| format!("user not found: {}", user_id))?;
        get_bound_udp_socket_id(&u.resources).ok_or_else(|| "udp socket not bound".to_string())?
    };
    let mut v: serde_json::Value =
        serde_json::from_str(&def.params).map_err(|e| format!("decode def.params: {e}"))?;
    if let Some(obj) = v.as_object_mut() {
        obj.entry("socket_id".to_string())
            .or_insert(serde_json::Value::Number(sock_id.into()));
    }
    def.params = serde_json::to_string(&v).map_err(|e| format!("encode def.params: {e}"))?;
    Ok(())
}

fn get_bound_udp_socket_id(resources: &serde_json::Value) -> Option<u64> {
    resources
        .get("_bound")
        .and_then(|b| b.get("udp_socket_id"))
        .and_then(|v| v.as_u64())
}

fn get_bound_udp_owner_id(resources: &serde_json::Value) -> Option<String> {
    resources
        .get("_bound")
        .and_then(|b| b.get("udp_owner_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn set_bound_udp_socket_id(resources: &mut serde_json::Value, sock_id: u64) {
    if !resources.is_object() {
        *resources = serde_json::json!({});
    }
    let obj = resources.as_object_mut().unwrap();
    let bound = obj
        .entry("_bound".to_string())
        .or_insert(serde_json::json!({}));
    if let Some(bobj) = bound.as_object_mut() {
        bobj.insert(
            "udp_socket_id".to_string(),
            serde_json::Value::Number(sock_id.into()),
        );
    }
}

fn set_bound_udp_owner_id(resources: &mut serde_json::Value, owner_id: &str) {
    if !resources.is_object() {
        *resources = serde_json::json!({});
    }
    let obj = resources.as_object_mut().unwrap();
    let bound = obj
        .entry("_bound".to_string())
        .or_insert(serde_json::json!({}));
    if let Some(bobj) = bound.as_object_mut() {
        bobj.insert(
            "udp_owner_id".to_string(),
            serde_json::Value::String(owner_id.to_string()),
        );
    }
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
    if let Ok(mut jobs) = SEND_JOBS.lock() {
        jobs.retain(|_, j| j.req.user_id != user_id);
    }

    // 5) clear sock ctx mappings
    clear_sock_ctx_for_user(user_id);

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

fn on_action_result_event(
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

    let (_ver, sc_arc, wf_idx) = get_user_scenario_ctx(user_id)?;
    let sc = sc_arc.as_ref();

    let status_lc = status.to_ascii_lowercase();
    let success = status_lc.contains("success");
    let timeout = status_lc.contains("timeout");
    let eval_ctx = serde_json::json!({
        "event": "scheduler.action-result",
        "reason": if success { "success" } else if timeout { "timeout" } else { "failed" },
        "user_id": user_id,
        "task_id": task_id,
        "action_id": ev.action_id.as_deref().unwrap_or(""),
        "status": status,
        "detail": detail,
        "exports": exports.clone().unwrap_or(serde_json::Value::Null),
    });

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
            "scheduler.timer.retry",
            now_ms().saturating_add(after),
            user_id,
            task_id,
            ev.action_id.as_deref(),
            serde_json::json!({"user_id": user_id, "task_id": task_id, "left": retries_left}),
        );
    }

    // workflow 推进：成功/失败/超时 都可以有不同分支；但若还有 retry，则延后失败/超时分支推进
    let reason = if success {
        "success"
    } else if timeout {
        "timeout"
    } else {
        "failed"
    };
    // should advance workflow edges only when:
    // - success and no jump_step
    // - failure/timeout and no retry and no jump_step
    let should_advance =
        ((success) && jump_step.is_none()) || ((!success) && !need_retry && jump_step.is_none());
    let effects = {
        let mut sm = STATE_MACHINE
            .lock()
            .map_err(|_| "lock state-machine".to_string())?;
        if let Some(ns) = jump_step {
            sm.set_step(user_id, task_id, ns, now_ms());
        }
        sm.apply(
            sc,
            &wf_idx,
            now_ms(),
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
    apply_sm_effects(effects)?;
    if should_advance {
        maybe_finish_user(ctx, user_id)?;
    }

    Ok(())
}

// ----------------- 模板展开与 Action 构造 -----------------
#[allow(dead_code)]
fn build_action_def_with_ctx(
    action: &Action,
    ctx: &TemplateContext,
    user_id: Option<&str>,
    task_id: Option<&str>,
) -> Result<(ActionDef, ActionContext), String> {
    let expanded = render_value(&action.with, ctx)?;
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

fn render_value(
    value: &serde_json::Value,
    ctx: &TemplateContext,
) -> Result<serde_json::Value, String> {
    match value {
        serde_json::Value::String(s) => Ok(serde_json::Value::String(render_str(s, ctx)?)),
        serde_json::Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                out.push(render_value(v, ctx)?);
            }
            Ok(serde_json::Value::Array(out))
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), render_value(v, ctx)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

fn render_str(input: &str, ctx: &TemplateContext) -> Result<String, String> {
    // 简单占位符：{{ path.to.value }}，按 vars > resources > exports 查找
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        let (prefix, rem) = rest.split_at(start);
        out.push_str(prefix);
        if let Some(end_rel) = rem.find("}}") {
            let (inside_with_brace, after) = rem.split_at(end_rel + 2);
            let key = inside_with_brace
                .trim_start_matches("{{")
                .trim_end_matches("}}")
                .trim();
            let val = lookup_path(key, ctx).unwrap_or_else(|| "".to_string());
            out.push_str(&val);
            rest = after;
        } else {
            out.push_str(rem);
            rest = "";
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn lookup_path(path: &str, ctx: &TemplateContext) -> Option<String> {
    let parts: Vec<&str> = path.split('.').collect();
    for src in [&ctx.vars, &ctx.resources, &ctx.exports] {
        if let Some(val) = get_path(src, &parts) {
            return Some(val);
        }
    }
    None
}

fn get_path<'a>(val: &'a serde_json::Value, parts: &[&str]) -> Option<String> {
    let mut cur = val;
    for p in parts {
        cur = cur.get(*p)?;
    }
    match cur {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        other => serde_json::to_string(other).ok(),
    }
}

#[derive(Clone)]
struct SendJob {
    req: SendRequest,
    next_send_ms: u64,
    total_sent: u32,
    last_sent_time_ms: Option<u64>,
    last_error: Option<String>,
}

static SEND_JOBS: Lazy<Mutex<HashMap<String, SendJob>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn le_u16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

fn le_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

fn to_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for byte in data {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", byte);
    }
    s
}

/// 参考 packet-engine 的 drain_rx_ring：读取 desc ring，生成事件到 eventbus。
fn drain_rx_ring(desc_mem: Vec<u8>, payload_mem: Vec<u8>) -> u32 {
    if desc_mem.len() < CONTROL_LEN {
        return 0;
    }

    let magic = le_u32(&desc_mem[0..4]);
    let version = le_u16(&desc_mem[4..6]);
    if magic != NTX_MAGIC || version != NTX_VERSION {
        return 0;
    }

    let desc_capacity = le_u32(&desc_mem[8..12]) as usize;
    let mut head = le_u32(&desc_mem[12..16]) as usize;
    let tail = le_u32(&desc_mem[16..20]) as usize;

    if desc_capacity == 0 {
        return 0;
    }

    let mut consumed: u32 = 0;

    while head != tail {
        let idx = head % desc_capacity;
        let base = DESCS_OFF + idx * DESC_LEN;
        if base + DESC_LEN > desc_mem.len() {
            break;
        }

        let desc = &desc_mem[base..base + DESC_LEN];
        let sock_id = le_u64(&desc[0..8]);
        let payload_off = le_u32(&desc[8..12]) as usize;
        let payload_len = le_u32(&desc[12..16]) as usize;

        if payload_off + payload_len <= payload_mem.len() {
            let payload = &payload_mem[payload_off..payload_off + payload_len];

            // 根据 sock_id 查找 user/task/action 关联，并刷新 last_seen_ms
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let ctx = {
                let mut guard = SOCK_CTX.lock().ok();
                guard
                    .as_mut()
                    .and_then(|map| map.get_mut(&sock_id))
                    .map(|c| {
                        c.last_seen_ms = now_ms;
                        c.clone()
                    })
            };

            publish_packet_event(sock_id, payload, ctx.as_ref(), now_ms);
        }

        head = head.wrapping_add(1);
        consumed += 1;
        if consumed >= MAX_CONSUME {
            break;
        }
    }

    consumed
}

fn publish_packet_event(sock_id: u64, payload: &[u8], ctx: Option<&SockCtx>, now_ms: u64) {
    let id = format!("rx-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed));
    let seq = PACKET_RX_SEQ.fetch_add(1, Ordering::Relaxed);

    let json_payload = serde_json::json!({
        "sock_id": sock_id,
        "seq": seq,
        "len": payload.len(),
        "payload_hex": to_hex(payload),
        "ts_ms": now_ms,
    })
    .to_string();

    // 注意：packet.rx 只负责产生事件；状态迁移应由 workflow 的 wait 节点触发器决定。

    let _ = ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
        id,
        kind: "packet.rx".to_string(),
        user_id: ctx.and_then(|c| c.user_id.clone()),
        task_id: ctx.and_then(|c| c.task_id.clone()),
        action_id: ctx.and_then(|c| c.action_id.clone()),
        payload: json_payload,
        correlation_id: ctx.and_then(|c| c.correlation_id.clone()),
        timestamp_ms: now_ms,
    });
}

fn handle_tx_request(payload_json: &str, correlation_id: Option<&str>) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct TxReq {
        sock_id: u64,
        #[serde(default)]
        payload: Option<String>,
        #[serde(default)]
        payload_hex: Option<String>,
        #[serde(default)]
        payload_bytes: Option<Vec<u8>>,
        #[serde(default)]
        user_id: Option<String>,
        #[serde(default)]
        task_id: Option<String>,
        #[serde(default)]
        action_id: Option<String>,
    }

    fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
        let mut t = s.trim().to_ascii_lowercase();
        if let Some(rest) = t.strip_prefix("0x") {
            t = rest.to_string();
        }
        let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        if t.len() % 2 != 0 {
            return Err("payload_hex length must be even".to_string());
        }
        let bytes = t
            .as_bytes()
            .chunks(2)
            .map(|pair| {
                let hi = pair[0] as char;
                let lo = pair[1] as char;
                let hex = [hi, lo].iter().collect::<String>();
                u8::from_str_radix(&hex, 16).map_err(|_| format!("invalid hex byte: {hex}"))
            })
            .collect::<Result<Vec<u8>, String>>()?;
        Ok(bytes)
    }

    let req: TxReq = serde_json::from_str(payload_json)
        .map_err(|e| format!("parse tx-request payload json: {e}"))?;

    let payload: Vec<u8> = if let Some(b) = req.payload_bytes.clone() {
        b
    } else if let Some(h) = req.payload_hex.as_deref() {
        decode_hex(h)?
    } else if let Some(s) = req.payload.clone() {
        s.into_bytes()
    } else {
        return Err("missing payload: expected payload / payload_hex / payload_bytes".to_string());
    };

    send_udp(
        req.sock_id,
        &payload,
        req.user_id.as_deref(),
        req.task_id.as_deref(),
        req.action_id.as_deref(),
        correlation_id,
    )
}

fn on_send_schedule_request(ev: &ntx::scenario_eventbus::event_bus::Event) -> Result<(), String> {
    // Parse executor-published JSON payload and enqueue job.
    // Payload is expected to be compatible with core-types SendRequest fields.
    #[derive(serde::Deserialize)]
    struct SendScheduleReq {
        request_id: String,
        user_id: String,
        task_id: String,
        socket_id: u64,
        #[serde(default)]
        max_count: Option<u32>,
        #[serde(default)]
        timeout_ms: Option<u64>,
        #[serde(default)]
        payload_bytes: Option<Vec<u8>>,
        #[serde(default)]
        schedule: serde_json::Value,
    }

    fn parse_schedule(v: &serde_json::Value) -> Result<SendSchedule, String> {
        let mode = v
            .get("mode")
            .and_then(|x| x.as_str())
            .unwrap_or("once")
            .trim()
            .to_ascii_lowercase();
        match mode.as_str() {
            "once" => Ok(SendSchedule::Once),
            "periodic" => {
                let interval_ms = v
                    .get("interval_ms")
                    .and_then(|x| x.as_u64())
                    .ok_or_else(|| "missing schedule.interval_ms".to_string())?;
                let start_delay_ms = v.get("start_delay_ms").and_then(|x| x.as_u64());
                Ok(SendSchedule::Periodic(
                    crate::ntx::core_types::types::PeriodicSchedule {
                        interval_ms,
                        start_delay_ms,
                    },
                ))
            }
            "timetable" => {
                let ts = v
                    .get("timestamps_ms")
                    .and_then(|x| x.as_array())
                    .ok_or_else(|| "missing schedule.timestamps_ms".to_string())?;
                let mut out: Vec<u64> = Vec::with_capacity(ts.len());
                for x in ts {
                    let n = x
                        .as_u64()
                        .ok_or_else(|| "timestamps_ms must be u64".to_string())?;
                    out.push(n);
                }
                Ok(SendSchedule::Timetable(
                    crate::ntx::core_types::types::TimetableSchedule { timestamps_ms: out },
                ))
            }
            "rate-limited" | "rate_limited" | "ratelimited" => {
                let pps = v
                    .get("pps")
                    .and_then(|x| x.as_u64())
                    .ok_or_else(|| "missing schedule.pps".to_string())?;
                let burst_size = v.get("burst_size").and_then(|x| x.as_u64());
                Ok(SendSchedule::RateLimited(
                    crate::ntx::core_types::types::RateLimitedSchedule {
                        pps: pps as u32,
                        burst_size: burst_size.map(|b| b as u32),
                    },
                ))
            }
            other => Err(format!("unsupported schedule mode: {other}")),
        }
    }

    let req: SendScheduleReq =
        serde_json::from_str(&ev.payload).map_err(|e| format!("parse payload json: {e}"))?;

    let schedule = parse_schedule(&req.schedule)?;

    let payload = req
        .payload_bytes
        .ok_or_else(|| "missing payload_bytes for send.schedule-request".to_string())?;

    let core_req = SendRequest {
        request_id: req.request_id.clone(),
        user_id: req.user_id.clone(),
        task_id: req.task_id.clone(),
        socket_id: req.socket_id,
        schedule,
        payload: Some(payload),
        payload_generator: None,
        max_count: req.max_count,
        timeout_ms: req.timeout_ms,
    };

    // basic validation
    if let SendSchedule::RateLimited(r) = &core_req.schedule {
        if r.pps == 0 {
            return Err("pps must be > 0".to_string());
        }
    }

    let now = now_ms();
    let next_ms = calc_initial_next_send_ms(&core_req, now);
    let job = SendJob {
        req: core_req.clone(),
        next_send_ms: next_ms,
        total_sent: 0,
        last_sent_time_ms: None,
        last_error: None,
    };
    if let Ok(mut map) = SEND_JOBS.lock() {
        map.insert(core_req.request_id.clone(), job);
    }

    // optional: publish a status/ack event
    publish_event_with_corr(
        "send.scheduled",
        Some(core_req.user_id.as_str()),
        Some(core_req.task_id.as_str()),
        ev.action_id.as_deref(),
        ev.correlation_id.as_deref(),
        serde_json::json!({
            "request_id": core_req.request_id,
            "socket_id": core_req.socket_id,
            "state": "pending",
            "next_send_ms": next_ms,
        }),
    );

    Ok(())
}

/// 在 socket 关闭时清理 sock_id 对应的上下文映射。
pub fn clear_sock_ctx_for_socket(sock_id: u64) {
    if let Ok(mut map) = SOCK_CTX.lock() {
        map.remove(&sock_id);
    }
}

/// 在 user 结束时清理该 user 相关的所有 sock 上下文。
pub fn clear_sock_ctx_for_user(user_id: &str) {
    if let Ok(mut map) = SOCK_CTX.lock() {
        map.retain(|_, ctx| ctx.user_id.as_deref() != Some(user_id));
    }
}

fn send_udp(
    sock_id: u64,
    payload: &[u8],
    user_id: Option<&str>,
    task_id: Option<&str>,
    action_id: Option<&str>,
    correlation_id: Option<&str>,
) -> Result<(), String> {
    let now_ms = now_ms();

    // 记录 sock 上下文用于后续 packet.rx 关联
    {
        if let Ok(mut map) = SOCK_CTX.lock() {
            map.insert(
                sock_id,
                SockCtx {
                    user_id: user_id.map(|s| s.to_string()),
                    task_id: task_id.map(|s| s.to_string()),
                    action_id: action_id.map(|s| s.to_string()),
                    correlation_id: correlation_id.map(|s| s.to_string()),
                    last_seen_ms: now_ms,
                },
            );
        }
    }

    // 注意：不在 packet.tx-request 处理路径里直接做 workflow 状态迁移；
    // Waiting 由 workflow edge（action -> wait）或 wait 节点触发器决定。

    let frame = udp_socket_control::build_reply(sock_id, payload)
        .map_err(|e| format!("build_reply failed: {:?}", e))?;
    udp_socket_control::tx(frame).map_err(|e| format!("tx failed: {:?}", e))?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn is_job_active(job: &SendJob) -> bool {
    matches!(
        job_state(job),
        SendRequestState::Pending | SendRequestState::Active
    )
}

fn job_state(job: &SendJob) -> SendRequestState {
    if job
        .last_error
        .as_ref()
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        return SendRequestState::Error;
    }

    if let Some(max) = job.req.max_count {
        if job.total_sent >= max {
            return SendRequestState::Completed;
        }
    }

    if job.total_sent > 0 {
        SendRequestState::Active
    } else {
        SendRequestState::Pending
    }
}

fn calc_initial_next_send_ms(req: &SendRequest, base_ms: u64) -> u64 {
    match &req.schedule {
        SendSchedule::Once => base_ms,
        SendSchedule::Periodic(p) => base_ms + p.start_delay_ms.unwrap_or(0),
        SendSchedule::Timetable(t) => base_ms + t.timestamps_ms.first().cloned().unwrap_or(0),
        SendSchedule::RateLimited(_) => base_ms,
    }
}

fn next_due_after(job: &SendJob, now_ms: u64) -> Option<u64> {
    match &job.req.schedule {
        SendSchedule::Once => None,
        SendSchedule::Periodic(p) => Some(now_ms + p.interval_ms),
        SendSchedule::Timetable(t) => {
            let idx = job.total_sent as usize;
            t.timestamps_ms
                .get(idx + 1)
                .map(|delta| now_ms.saturating_add(*delta))
        }
        SendSchedule::RateLimited(r) => {
            if r.pps == 0 {
                None
            } else {
                let interval = 1000u64 / (r.pps as u64);
                Some(now_ms + interval)
            }
        }
    }
}

// fn schedule_send_job(request: SendRequest) -> Result<String, String> {
//     if request.payload.is_none() && request.payload_generator.is_some() {
//         return Err("payload-generator not supported yet".into());
//     }
//     if request.payload.is_none() && request.payload_generator.is_none() {
//         return Err("missing payload".into());
//     }
//     if let SendSchedule::RateLimited(r) = &request.schedule {
//         if r.pps == 0 {
//             return Err("pps must be > 0".into());
//         }
//     }

//     let now = now_ms();
//     let next_ms = calc_initial_next_send_ms(&request, now);
//     let job = SendJob {
//         req: request.clone(),
//         next_send_ms: next_ms,
//         total_sent: 0,
//         last_sent_time_ms: None,
//         last_error: None,
//     };
//     if let Ok(mut map) = SEND_JOBS.lock() {
//         map.insert(request.request_id.clone(), job);
//     }
//     Ok(request.request_id)
// }

// fn cancel_send_job(request_id: &str) -> Result<(), String> {
//     if let Ok(mut map) = SEND_JOBS.lock() {
//         if map.remove(request_id).is_none() {
//             return Err(format!("request not found: {request_id}"));
//         }
//     }
//     Ok(())
// }
/*
fn query_send_status_job(request_id: &str) -> Result<SendStatus, String> {
    let job = SEND_JOBS
        .lock()
        .ok()
        .and_then(|m| m.get(request_id).cloned())
        .ok_or_else(|| format!("request not found: {request_id}"))?;

    Ok(SendStatus {
        request_id: request_id.to_string(),
        state: job_state(&job),
        total_sent: job.total_sent,
        last_sent_time_ms: job.last_sent_time_ms,
        next_send_time_ms: Some(job.next_send_ms),
        last_error: job.last_error,
    })
}
*/
fn tick_send_scheduler(now_ms: u64) {
    let mut to_send: Vec<String> = Vec::new();
    {
        if let Ok(jobs) = SEND_JOBS.lock() {
            for (id, job) in jobs.iter() {
                if is_job_active(job) && job.next_send_ms <= now_ms {
                    to_send.push(id.clone());
                }
            }
        }
    }

    for id in to_send {
        let mut remove = false;
        if let Some(mut job) = SEND_JOBS.lock().ok().and_then(|mut m| m.remove(&id)) {
            if let Err(e) = send_udp(
                job.req.socket_id,
                job.req.payload.as_deref().unwrap_or(&[]),
                Some(job.req.user_id.as_str()),
                Some(job.req.task_id.as_str()),
                None,
                None,
            ) {
                job.last_error = Some(e);
            } else {
                job.total_sent = job.total_sent.saturating_add(1);
                job.last_sent_time_ms = Some(now_ms);

                if let Some(max) = job.req.max_count {
                    if job.total_sent >= max {
                        remove = true;
                    }
                }

                if !remove {
                    if let Some(next) = next_due_after(&job, now_ms) {
                        job.next_send_ms = next;
                    } else {
                        remove = true;
                    }
                }
            }

            if !remove {
                if let Ok(mut m) = SEND_JOBS.lock() {
                    m.insert(id.clone(), job);
                }
            }
        }
    }
}
