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
use crate::ntx::core_types::types::{self, SendRequest};
struct ActionExecutorImpl;
use ntx_action_sdk::{ActionModule, ActionRequest, ActionRuntime};
// Macro-generate the remaining WIT glue (schedule parse + schedule publish)
// so this component can focus on business handlers.
ntx_action_sdk::define_wit_scheduler_send_glue!(
    types_mod = types,
    event_ty = crate::ntx::scenario_eventbus::event_bus::Event,
    publish_fn = crate::ntx::scenario_eventbus::event_bus::publish,
);

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

#[derive(serde::Deserialize)]
struct UdpScheduleSendParams {
    socket_id: u64,
    #[serde(default)]
    max_count: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    request_id: Option<String>,
}

// Macro-generate a schedule-send handler without UDP-specific SDK bindings.
// Here we configure it for the current action call: `udp.schedule-send`.
ntx_action_sdk::define_schedule_send_handler!(
    fn handle_udp_schedule_send,
    bus = WITEventBus,
    parse_params = |req: &ntx_action_sdk::ActionRequest| {
        ntx_action_sdk::parse_params::<UdpScheduleSendParams>(req)
    },
    build_request = |parsed: UdpScheduleSendParams,
                     user_id: &String,
                     task_id: &String,
                     schedule,
                     payload_bytes: Vec<u8>| {
        let request_id = parsed
            .request_id
            .unwrap_or_else(|| ntx_action_sdk::next_request_id("send"));

        Ok::<SendRequest, String>(SendRequest {
            request_id,
            user_id: user_id.clone(),
            task_id: task_id.clone(),
            socket_id: parsed.socket_id,
            schedule,
            payload: Some(payload_bytes),
            payload_generator: None,
            max_count: parsed.max_count,
            timeout_ms: parsed.timeout_ms,
        })
    },
    build_exports = |send_req: &SendRequest| {
        ntx_action_sdk::exports_json!({
            "request_id": send_req.request_id,
            "socket_id": send_req.socket_id,
            "scheduled": true,
        })
    },
    success_detail = |exports: &String| format!("udp.schedule-send ok request_id={}", exports),
);

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
