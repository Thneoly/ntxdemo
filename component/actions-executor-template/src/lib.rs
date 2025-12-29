//! actions-executor template (wasm32-wasip2 component)
//!
//! This shows the **framework mode** usage of `ntx-action-sdk`:
//! - `ActionModule` trait
//! - `EventBusAdapter`
//! - `ActionRuntime`

wit_bindgen::generate!({
    world: "ntx:scenario-actions-executor/action-executor-component@0.1.0",
    path: [
        "../wit/eventbus",
        "../wit/types",
        "../wit/actions-executor",
    ],
    generate_all,
    generate_unused_types: true,
    debug: true,
});

use ntx_action_sdk::{ActionModule, ActionRequest, ActionRuntime, FrameworkOutcome};

// Use generated core types module from wit-bindgen.
use crate::ntx::core_types::types;

struct ActionExecutorImpl;

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
                id: "ping".to_string(),
                call: "health.ping".to_string(),
                title: "Ping".to_string(),
                description: Some("Health check action".to_string()),
                tags: vec!["health".to_string()],
            },
        ]
    }

    pub fn describe_action(
        action_id: String,
    ) -> Result<exports::ntx::scenario_actions_executor::action_component::ActionSpec, String> {
        match action_id.as_str() {
            "ping" => {
                let input_schema_json = serde_json::json!({
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {}
                })
                .to_string();

                let defaults_json = serde_json::json!({}).to_string();

                Ok(
                    exports::ntx::scenario_actions_executor::action_component::ActionSpec {
                        id: "ping".to_string(),
                        call: "health.ping".to_string(),
                        title: "Ping".to_string(),
                        description: Some("Health check action".to_string()),
                        input_schema_json,
                        defaults_json: Some(defaults_json),
                        ui_schema_json: None,
                        examples_json: None,
                        capabilities: vec![],
                        executor_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    },
                )
            }
            _ => Err(format!("unknown action-id: {action_id}")),
        }
    }
}

// Adapter from WIT event bus to framework (macro-generated).
ntx_action_sdk::define_wit_event_bus!(
    WITEventBus,
    crate::ntx::scenario_eventbus::event_bus::Event,
    crate::ntx::scenario_eventbus::event_bus::publish
);

impl Default for WITEventBus {
    fn default() -> Self {
        WITEventBus
    }
}

/// Your action module: implement all call handlers here.
#[derive(Default)]
struct MyActions;

fn handle_ping(
    _rt: &ActionRuntime<'_, WITEventBus>,
    _req: &ActionRequest,
) -> Result<FrameworkOutcome, String> {
    Ok(FrameworkOutcome::success("pong")
        .with_exports_json(ntx_action_sdk::exports_json!({"pong": true})))
}

fn handle_http_prefix(
    _rt: &ActionRuntime<'_, WITEventBus>,
    req: &ActionRequest,
) -> Result<FrameworkOutcome, String> {
    Ok(FrameworkOutcome::failed(format!(
        "http action not implemented: {}",
        req.call
    )))
}

fn handle_fallback(
    rt: &ActionRuntime<'_, WITEventBus>,
    req: &ActionRequest,
) -> Result<FrameworkOutcome, String> {
    Ok(rt.unknown_action(&req.call))
}

impl ActionModule<WITEventBus> for MyActions {
    fn handle(
        &self,
        rt: &ActionRuntime<'_, WITEventBus>,
        req: &ActionRequest,
    ) -> Result<FrameworkOutcome, String> {
        // Declarative routing: keep the structure consistent across components.
        // You can declare aliases and prefix routes here.
        ntx_action_sdk::routes!(rt, req, {
            alias ["ping", "health.ping"] => handle_ping,

            prefix "http." => handle_http_prefix,

            _ => handle_fallback,
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
        "[actions-executor-template] execute id={} call={} user={:?} task={:?}",
        action.id, action.call, user_id, task_id
    );
}

fn log_after_dispatch(
    (action, _ctx, _req, out): (
        &types::ActionDef,
        &Option<types::ActionContext>,
        &ntx_action_sdk::ActionRequest,
        &ntx_action_sdk::FrameworkOutcome,
    ),
) {
    println!(
        "[actions-executor-template] outcome id={} call={} status={:?}",
        action.id, action.call, out.status
    );
}

ntx_action_sdk::define_wit_component_entry_minimal!(
    impl_ty = ActionExecutorImpl,
    guest_trait = exports::ntx::scenario_actions_executor::action_component::Guest,
    types_mod = (types),
    bus_ty = WITEventBus,
    module_ty = MyActions,
    before_dispatch_tuple = log_before_dispatch,
    after_dispatch_tuple = log_after_dispatch,
);

export!(ActionExecutorImpl);
