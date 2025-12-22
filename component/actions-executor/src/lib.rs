//! actions-executor 组件骨架（wasm32-wasip2）。
//! 当前仅回显输入，便于后续接入真实协议执行。

wit_bindgen::generate!({
    world: "scheduler:actions-executor/action-executor-component@0.2.0",
    path: [
        "../wit/core-types",
        "../wit/eventbus",
        "../wit/protocol",
    ],
    generate_all,
    debug: true,
});

struct ActionExecutorImpl;

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};

static EVENT_SEQ: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(1));

fn next_event_id() -> String {
    let n = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("ae-{}", n)
}

impl exports::scheduler::actions_executor::action_component::Guest for ActionExecutorImpl {
    fn init_component() -> Result<(), String> {
        println!("[actions-executor] init-component");
        Ok(())
    }

    fn execute_action(
        action: scheduler::core_types::types::ActionDef,
    ) -> Result<scheduler::core_types::types::ActionOutcome, String> {
        println!(
            "[actions-executor] execute action id={} call={} user={:?} task={:?}",
            action.id, action.call, action.user_id, action.task_id
        );

        match action.call.as_str() {
            // 对齐 packet-engine，但不直接调用 host：发布事件让 scheduler/host 侧处理。
            "udp.send-reply" => {
                let params: serde_json::Value = serde_json::from_str(&action.with_params)
                    .map_err(|e| format!("parse with_params as json: {e}"))?;
                let sock_id = params
                    .get("socket_id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| "missing socket_id (u64)".to_string())?;
                let payload = params
                    .get("payload")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();

                // 通过 eventbus 通知 scheduler/host 进行真正发包
                let event_id = next_event_id();
                let payload_json = serde_json::json!({
                    "sock_id": sock_id,
                    "payload": payload,
                    "action_id": action.id,
                    "task_id": action.task_id,
                    "user_id": action.user_id,
                })
                .to_string();
                scheduler::event_bus::event_bus::publish(&scheduler::event_bus::event_bus::Event {
                    id: event_id,
                    kind: "packet.tx-request".to_string(),
                    user_id: action.user_id.clone(),
                    task_id: action.task_id.clone(),
                    action_id: Some(action.id.clone()),
                    payload: payload_json,
                    correlation_id: None,
                    timestamp_ms: 0,
                })
                .map_err(|e| format!("publish tx-request failed: {e}"))?;

                Ok(scheduler::core_types::types::ActionOutcome {
                    status: scheduler::core_types::types::ActionStatus::Success,
                    detail: Some(format!(
                        "udp.send-reply delegated socket_id={} len={}",
                        sock_id,
                        payload.len()
                    )),
                    latency_ms: None,
                })
            }
            _ => Ok(scheduler::core_types::types::ActionOutcome {
                status: scheduler::core_types::types::ActionStatus::Success,
                detail: Some("stub executed".to_string()),
                latency_ms: None,
            }),
        }
    }

    fn release_component() -> Result<(), String> {
        println!("[actions-executor] release-component");
        Ok(())
    }
}

export!(ActionExecutorImpl);
