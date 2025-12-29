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

struct ActionExecutorImpl;

// Adapter from WIT event bus to framework (macro-generated).
ntx_action_sdk::define_wit_event_bus!(
    WITEventBus,
    ntx::scenario_eventbus::event_bus::Event,
    ntx::scenario_eventbus::event_bus::publish
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
        &ntx::core_types::types::ActionDef,
        &Option<ntx::core_types::types::ActionContext>,
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
        &ntx::core_types::types::ActionDef,
        &Option<ntx::core_types::types::ActionContext>,
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
    guest_trait = crate::exports::ntx::scenario_actions_executor::action_component::Guest,
    types_mod = (ntx::core_types::types),
    bus_ty = WITEventBus,
    module_ty = MyActions,
    before_dispatch_tuple = log_before_dispatch,
    after_dispatch_tuple = log_after_dispatch,
);

export!(ActionExecutorImpl);
