//! Scenario configuration types.
//!
//! Kept in a standalone module to make `lib.rs` smaller and easier to navigate.

use serde::{Deserialize, Serialize};

/// Strongly typed scenario config (minimal).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Scenario {
    #[serde(default)]
    pub(crate) workbook: Workbook,
    #[serde(default)]
    pub(crate) actions: Actions,
    #[serde(default)]
    pub(crate) workflows: Workflow,
    #[serde(default)]
    pub(crate) load: Load,
    #[serde(default)]
    pub(crate) user_resources: UserResources,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct Workbook {
    #[serde(default)]
    pub(crate) resources: Vec<Resource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Resource {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) r#type: ResourceType,
    #[serde(default)]
    pub(crate) properties: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ResourceType {
    UdpEndpoint,
    /// Forward-compat: allow new resource types without breaking deserialization.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct Actions {
    #[serde(default)]
    pub(crate) actions: Vec<Action>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Action {
    pub(crate) id: String,
    pub(crate) call: ActionCall,
    #[serde(default)]
    pub(crate) with: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ActionCall {
    UdpSendReply,
    /// Forward-compat: allow new actions without breaking deserialization.
    #[serde(other)]
    Other,
}

impl ActionCall {
    pub(crate) fn as_call_str(&self) -> &'static str {
        match self {
            ActionCall::UdpSendReply => "udp.send-reply",
            ActionCall::Other => "other",
        }
    }

    pub(crate) fn is_udp(&self) -> bool {
        matches!(self, ActionCall::UdpSendReply)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct Workflow {
    #[serde(default)]
    pub(crate) nodes: Vec<WorkflowNodeDef>,
}

/// Workflow node kind (parsed from scenario YAML field `type`).
///
/// We keep it strongly typed to avoid stringly-typed comparisons across the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NodeKind {
    Action,
    Wait,
    End,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WorkflowNodeDef {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) kind: NodeKind,
    #[serde(default)]
    pub(crate) action: Option<String>,
    /// Multi-step action: run action ids sequentially.
    #[serde(default)]
    pub(crate) actions: Option<Vec<String>>,
    /// Stronger step semantics.
    #[serde(default)]
    pub(crate) steps: Option<Vec<NodeStepDef>>,
    /// Scheduling priority (higher = earlier).
    #[serde(default)]
    pub(crate) priority: Option<i32>,
    /// wait node: event + match spec.
    #[serde(default)]
    pub(crate) on: Option<WaitOnSpec>,
    #[serde(default)]
    pub(crate) edges: Vec<WorkflowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeStepDef {
    pub(crate) action: String,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) retry: Option<RetryDef>,
    #[serde(default)]
    pub(crate) on_failed_step: Option<u32>,
    #[serde(default)]
    pub(crate) on_timeout_step: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RetryDef {
    #[serde(default)]
    pub(crate) max: i64,
    #[serde(default)]
    pub(crate) backoff_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WaitOnSpec {
    pub(crate) event: WaitEvent,
    #[serde(default)]
    pub(crate) r#match: WaitMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum WaitEvent {
    PacketRx,
    /// Forward-compat: allow new events without breaking deserialization.
    #[serde(other)]
    Other,
}

/// Strongly typed match spec for `wait` nodes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WaitMatch {
    #[serde(default)]
    pub(crate) action_id: Option<String>,
    #[serde(default)]
    pub(crate) task_id: Option<String>,
    #[serde(default)]
    pub(crate) sock_id: Option<u64>,
    #[serde(default)]
    pub(crate) len: Option<u64>,
    #[serde(default)]
    pub(crate) payload_hex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WorkflowEdge {
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) label: Option<String>,
    #[serde(default)]
    pub(crate) trigger: Option<TriggerSpec>,
}

/// Strongly typed workflow edge trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TriggerSpec {
    /// Alias keys: `on` / `event` / `status` in YAML.
    #[serde(default, flatten)]
    pub(crate) when: Option<TriggerWhen>,
    #[serde(default)]
    pub(crate) condition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, untagged)]
pub(crate) enum TriggerWhen {
    On { on: TriggerReason },
    Event { event: TriggerReason },
    Status { status: TriggerReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TriggerReason {
    Success,
    Failed,
    Timeout,
    PacketRx,
    /// Fallback for future extensions (keeps forward-compat).
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct Load {
    #[serde(default)]
    pub(crate) ramp_up: RampUp,
    #[serde(default)]
    pub(crate) user_lifetime: UserLifetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RampUp {
    #[serde(default)]
    pub(crate) phases: Vec<RampPhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RampPhase {
    pub(crate) at_second: u64,
    pub(crate) spawn_users: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct UserLifetime {
    #[serde(default = "default_mode")]
    pub(crate) mode: UserLifetimeMode,
    #[serde(default)]
    pub(crate) iterations: Option<u64>,
    #[serde(default)]
    pub(crate) think_time: Option<String>,
    /// Per-user concurrency limit (Running task count).
    #[serde(default)]
    pub(crate) max_concurrency: Option<u32>,
}

fn default_mode() -> UserLifetimeMode {
    UserLifetimeMode::Once
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum UserLifetimeMode {
    Once,
    Loop,
    /// Forward-compat.
    #[serde(other)]
    Other,
}

impl Default for UserLifetimeMode {
    fn default() -> Self {
        UserLifetimeMode::Once
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct UserResources {
    #[serde(default)]
    pub(crate) ip_binding: Option<IpBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IpBinding {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) pool_id: Option<String>,
    #[serde(default)]
    pub(crate) strategy: Option<String>,
    #[serde(default)]
    pub(crate) release_on: Option<String>,
}
