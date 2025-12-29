//! Schedule-send action handler glue.
//!
//! This module owns the `udp.schedule-send` handler and any extra glue needed
//! to publish "send schedule" events through the component's WIT event bus.

use crate::ntx::core_types::types;

// Macro-generate the remaining WIT glue (schedule parse + schedule publish)
// so the component can focus on business handlers.
ntx_action_sdk::define_wit_scheduler_send_glue!(
    types_mod = types,
    event_ty = crate::ntx::scenario_eventbus::event_bus::Event,
    publish_fn = crate::ntx::scenario_eventbus::event_bus::publish,
);

use crate::ntx::core_types::types::SendRequest;

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
    pub(crate) fn handle_udp_schedule_send,
    bus = crate::WITEventBus,
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
