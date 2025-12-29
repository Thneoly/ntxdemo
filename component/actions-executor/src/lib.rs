//! actions-executor 组件骨架（wasm32-wasip2）。
//! 当前仅回显输入，便于后续接入真实协议执行。

wit_bindgen::generate!({
    world: "ntx:scenario-actions-executor/action-executor-component@0.1.0",
    path: [
        "../wit/eventbus",
        "../wit/types",
        "../wit/actions-executor",
    ],
    generate_all,
    generate_unused_types:true,
    debug: true,
});
use crate::ntx::core_types::types;
struct ActionExecutorImpl;
use ntx_action_sdk::{ActionModule, ActionRequest, ActionRuntime};

mod schedule_send;

impl ActionExecutorImpl {
    // NOTE: do NOT implement the generated `Guest` trait here; the SDK macro does that.
    // We expose these as inherent fns so the macro glue can call them.
    pub fn schema_version() -> u32 {
        1
    }

    pub fn list_actions(
    ) -> Vec<exports::ntx::scenario_actions_executor::action_component::ActionSummary> {
        vec![
            exports::ntx::scenario_actions_executor::action_component::ActionSummary {
                id: "udp-send-reply".to_string(),
                call: "udp.send-reply".to_string(),
                title: "UDP Send (no-wait)".to_string(),
                description: Some(
                    "Publish packet.tx-request and return immediately; scheduler handles wait/timeout/retry."
                        .to_string(),
                ),
                tags: vec!["udp".to_string(), "tx".to_string()],
            },
            exports::ntx::scenario_actions_executor::action_component::ActionSummary {
                id: "udp-schedule-send".to_string(),
                call: "udp.schedule-send".to_string(),
                title: "UDP Schedule Send".to_string(),
                description: Some(
                    "Publish packet.send-schedule-request; scheduler executes schedule (periodic/timetable/rate-limited)."
                        .to_string(),
                ),
                tags: vec!["udp".to_string(), "tx".to_string(), "schedule".to_string()],
            },
        ]
    }

    pub fn describe_action(
        action_id: String,
    ) -> Result<exports::ntx::scenario_actions_executor::action_component::ActionSpec, String> {
        match action_id.as_str() {
            "udp-send-reply" => {
                let input_schema_json = serde_json::json!({
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {
                        "socket_id": {"type": "integer", "minimum": 0},
                        "payload_hex": {"type": "string", "description": "hex string payload"},
                        "payload_utf8": {"type": "string", "description": "utf8 payload (alternative to payload_hex)"},
                        "payload_base64": {"type": "string", "description": "base64 payload (alternative)"},
                        "timeout_ms": {"type": "integer", "minimum": 0}
                    },
                    "required": ["socket_id"],
                })
                .to_string();

                let defaults_json = serde_json::json!({
                    "timeout_ms": 1000,
                    "payload_utf8": "hello"
                })
                .to_string();

                Ok(exports::ntx::scenario_actions_executor::action_component::ActionSpec {
                    id: "udp-send-reply".to_string(),
                    call: "udp.send-reply".to_string(),
                    title: "UDP Send (no-wait)".to_string(),
                    description: Some(
                        "Publish packet.tx-request and return immediately; scheduler handles wait/timeout/retry."
                            .to_string(),
                    ),
                    input_schema_json,
                    defaults_json: Some(defaults_json),
                    ui_schema_json: None,
                    examples_json: None,
                    capabilities: vec![
                        exports::ntx::scenario_actions_executor::action_component::ActionCapability::EmitsPacketTxRequest,
                        exports::ntx::scenario_actions_executor::action_component::ActionCapability::NeedsUserResources,
                    ],
                    executor_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                })
            }
            "udp-schedule-send" => {
                // This action takes a schedule-like JSON object and emits a scheduler send request.
                let input_schema_json = serde_json::json!({
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {
                        "socket_id": {"type": "integer", "minimum": 0},

                        // payload fields are parsed by SDK helper `parse_payload_spec`.
                        "payload_hex": {"type": "string"},
                        "payload_utf8": {"type": "string"},
                        "payload_base64": {"type": "string"},
                        "payload_bytes": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 255}},

                        // schedule fields are parsed by SDK helper `parse_schedule_like`.
                        "mode": {"type": "string", "enum": ["periodic", "timetable", "rate-limited"]},
                        "interval_ms": {"type": "integer", "minimum": 1},
                        "start_delay_ms": {"type": "integer", "minimum": 0},
                        "timestamps_ms": {"type": "array", "items": {"type": "integer", "minimum": 0}},
                        "rate_per_sec": {"type": "number", "minimum": 0},
                        "burst": {"type": "integer", "minimum": 0},

                        "max_count": {"type": "integer", "minimum": 0},
                        "timeout_ms": {"type": "integer", "minimum": 0},
                        "request_id": {"type": "string"}
                    },
                    "required": ["socket_id", "mode"],
                })
                .to_string();

                let defaults_json = serde_json::json!({
                    "mode": "periodic",
                    "interval_ms": 1000,
                    "start_delay_ms": 0,
                    "payload_utf8": "hello",
                    "timeout_ms": 30000,
                })
                .to_string();

                Ok(exports::ntx::scenario_actions_executor::action_component::ActionSpec {
                    id: "udp-schedule-send".to_string(),
                    call: "udp.schedule-send".to_string(),
                    title: "UDP Schedule Send".to_string(),
                    description: Some(
                        "Publish packet.send-schedule-request; scheduler executes schedule (periodic/timetable/rate-limited)."
                            .to_string(),
                    ),
                    input_schema_json,
                    defaults_json: Some(defaults_json),
                    ui_schema_json: None,
                    examples_json: None,
                    capabilities: vec![
                        exports::ntx::scenario_actions_executor::action_component::ActionCapability::EmitsPacketTxRequest,
                        exports::ntx::scenario_actions_executor::action_component::ActionCapability::NeedsUserResources,
                    ],
                    executor_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                })
            }
            _ => Err(format!("unknown action-id: {action_id}")),
        }
    }
}
ntx_action_sdk::define_wit_event_bus!(
    WITEventBus,
    ntx::scenario_eventbus::event_bus::Event,
    ntx::scenario_eventbus::event_bus::publish
);

#[derive(Default)]
struct ActionsExecutorModule;

impl Default for WITEventBus {
    fn default() -> Self {
        WITEventBus
    }
}

// Schedule-send glue + handler live in their own module.
// This keeps `lib.rs` focused on catalog + routing + entrypoint.
use schedule_send::handle_udp_schedule_send;

fn log_after_dispatch(
    (action, ctx, _req, out): (
        &types::ActionDef,
        &Option<types::ActionContext>,
        &ntx_action_sdk::ActionRequest,
        &ntx_action_sdk::FrameworkOutcome,
    ),
) {
    let user_id: Option<String> = ctx.as_ref().and_then(|c| c.user_id.clone());
    let task_id = ctx.as_ref().and_then(|c| c.task_id.clone());
    println!(
        "[actions-executor] outcome id={} call={} status={:?} user={:?} task={:?}",
        action.id, action.call, out.status, user_id, task_id
    );
}

impl ActionsExecutorModule {
    fn handle_udp_send(
        rt: &ActionRuntime<'_, WITEventBus>,
        req: &ActionRequest,
    ) -> Result<ntx_action_sdk::FrameworkOutcome, String> {
        let sock_id = req
            .params_json
            .get("socket_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "missing socket_id (u64)".to_string())?;

        let payload_spec = ntx_action_sdk::parse_payload_spec(&req.params_json)?;
        rt.publish_tx_request(sock_id, payload_spec.clone(), &req.id)?;

        let exports = ntx_action_sdk::exports_json!({
            "socket_id": sock_id,
            "action_call": req.call,
            "note": "tx delegated; rx (if any) is handled via scheduler packet.rx + workflow wait node",
        });

        Ok(ntx_action_sdk::FrameworkOutcome::success(format!(
            "{} delegated socket_id={}",
            req.call, sock_id
        ))
        .with_exports_json(exports))
    }

    fn handle_udp_send_recv(
        rt: &ActionRuntime<'_, WITEventBus>,
        req: &ActionRequest,
    ) -> Result<ntx_action_sdk::FrameworkOutcome, String> {
        let sock_id = req
            .params_json
            .get("socket_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "missing socket_id (u64)".to_string())?;

        let payload_spec = ntx_action_sdk::parse_payload_spec(&req.params_json)?;
        rt.publish_tx_request(sock_id, payload_spec, &req.id)?;

        let exports = ntx_action_sdk::exports_json!({
            "socket_id": sock_id,
            "action_call": req.call,
            "note": "tx delegated; rx/timeout/retry must be handled by scheduler state-machine (wait node + timer events)",
        });

        Ok(ntx_action_sdk::FrameworkOutcome::success(format!(
            "udp.send-recv delegated (no-wait) socket_id={}",
            sock_id
        ))
        .with_exports_json(exports))
    }

    fn handle_not_implemented(
        _rt: &ActionRuntime<'_, WITEventBus>,
        req: &ActionRequest,
    ) -> Result<ntx_action_sdk::FrameworkOutcome, String> {
        Ok(ntx_action_sdk::FrameworkOutcome::failed(format!(
            "action not implemented yet: {}",
            req.call
        )))
    }

    fn handle_fallback(
        rt: &ActionRuntime<'_, WITEventBus>,
        req: &ActionRequest,
    ) -> Result<ntx_action_sdk::FrameworkOutcome, String> {
        Ok(rt.unknown_action(&req.call))
    }
}

impl ActionModule<WITEventBus> for ActionsExecutorModule {
    fn handle(
        &self,
        rt: &ActionRuntime<'_, WITEventBus>,
        req: &ActionRequest,
    ) -> Result<ntx_action_sdk::FrameworkOutcome, String> {
        ntx_action_sdk::routes!(rt, req, {
            alias ["udp.send", "udp.send-reply"] => Self::handle_udp_send,
            "udp.send-recv" => Self::handle_udp_send_recv,
            "udp.schedule-send" => handle_udp_schedule_send,
            prefix "http." => Self::handle_not_implemented,
            prefix "tcp." => Self::handle_not_implemented,
            _ => Self::handle_fallback,
        })
    }
}

fn log_before_dispatch(
    (action, ctx, _req): (
        &types::ActionDef,
        &Option<types::ActionContext>,
        &ntx_action_sdk::ActionRequest,
    ),
) {
    let user_id: Option<String> = ctx.as_ref().and_then(|c| c.user_id.clone());
    let task_id = ctx.as_ref().and_then(|c| c.task_id.clone());
    println!(
        "[actions-executor] execute action id={} call={} user={:?} task={:?}",
        action.id, action.call, user_id, task_id
    );
}

// Replace the boilerplate `execute_action` glue with SDK macro.
// We keep module selection and outcome mapping customizable via closures.
ntx_action_sdk::define_wit_component_entry_minimal!(
    impl_ty = ActionExecutorImpl,
    guest_trait = exports::ntx::scenario_actions_executor::action_component::Guest,
    types_mod = (types),
    bus_ty = WITEventBus,
    module_ty = ActionsExecutorModule,
    before_dispatch_tuple = log_before_dispatch,
    after_dispatch_tuple = log_after_dispatch,
);

export!(ActionExecutorImpl);
