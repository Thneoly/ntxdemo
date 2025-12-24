//! actions-executor 组件骨架（wasm32-wasip2）。
//! 当前仅回显输入，便于后续接入真实协议执行。

wit_bindgen::generate!({
    world: "ntx:scenario-actions-executor/action-executor-component@0.1.0",
    path: [
        "../wit/host",
        "../wit/core-types",
        "../wit/eventbus",
        "../wit/actions-executor",
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

impl exports::ntx::scenario_actions_executor::action_component::Guest for ActionExecutorImpl {
    fn init_component() -> Result<(), String> {
        println!("[actions-executor] init-component");
        Ok(())
    }

    fn execute_action(
        action: ntx::scenario_types::types::ActionDef,
        ctx: Option<ntx::scenario_types::types::ActionContext>,
    ) -> Result<ntx::scenario_types::types::ActionOutcome, String> {
        let user_id = ctx.as_ref().and_then(|c| c.user_id.clone());
        let task_id = ctx.as_ref().and_then(|c| c.task_id.clone());
        let action_id = ctx.as_ref().and_then(|c| c.action_id.clone());

        println!(
            "[actions-executor] execute action id={} call={} user={:?} task={:?}",
            action.id, action.call, user_id, task_id
        );

        match action.call.as_str() {
            // 对齐 packet-engine，但不直接调用 host：发布事件让 scheduler/host 侧处理。
            "udp.send-reply" => {
                let params: serde_json::Value = serde_json::from_str(&action.params)
                    .map_err(|e| format!("parse params as json: {e}"))?;
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
                    "task_id": task_id,
                    "user_id": user_id,
                })
                .to_string();
                ntx::scenario_eventbus::event_bus::publish(
                    &ntx::scenario_eventbus::event_bus::Event {
                        id: event_id,
                        kind: "packet.tx-request".to_string(),
                        user_id: user_id.clone(),
                        task_id: task_id.clone(),
                        action_id: Some(action.id.clone()),
                        payload: payload_json,
                        correlation_id: None,
                        timestamp_ms: 0,
                    },
                )
                .map_err(|e| format!("publish tx-request failed: {e}"))?;

                Ok(ntx::scenario_types::types::ActionOutcome {
                    status: ntx::scenario_types::types::OutcomeStatus::Success,
                    detail: Some(format!(
                        "udp.send-reply delegated socket_id={} len={}",
                        sock_id,
                        payload.len()
                    )),
                    metrics: None,
                    exports: None,
                })
            }
            _ => Ok(ntx::scenario_types::types::ActionOutcome {
                status: ntx::scenario_types::types::OutcomeStatus::Success,
                detail: Some("stub executed".to_string()),
                metrics: None,
                exports: None,
            }),
        }
    }

    fn release_component() -> Result<(), String> {
        println!("[actions-executor] release-component");
        Ok(())
    }
}

export!(ActionExecutorImpl);
