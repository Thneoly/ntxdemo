//! actions-executor 组件骨架（wasm32-wasip2）。
//! 当前仅回显输入，便于后续接入真实协议执行。

wit_bindgen::generate!({
    world: "ntx:scenario-actions-executor/action-executor-component@0.1.0",
    path: [
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
use std::time::{SystemTime, UNIX_EPOCH};

static EVENT_SEQ: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(1));

fn next_event_id() -> String {
    let n = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("ae-{}", n)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
enum PayloadSpec {
    Text(String),
    Hex(String),
    Bytes(Vec<u8>),
}

fn parse_timeout_ms(params: &serde_json::Value) -> Option<u64> {
    params
        .get("timeout-ms")
        .or_else(|| params.get("timeout_ms"))
        .and_then(|v| v.as_u64())
}

fn min_deadline_ms(now: u64, from_ctx: Option<u64>, from_params: Option<u64>) -> Option<u64> {
    // ctx.deadline_ms is absolute; params timeout is relative (ms from now)
    let p_deadline = from_params.map(|t| now.saturating_add(t));
    match (from_ctx, p_deadline) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn parse_payload_spec(params: &serde_json::Value) -> Result<PayloadSpec, String> {
    if let Some(arr) = params.get("payload_bytes").and_then(|v| v.as_array()) {
        let mut out = Vec::with_capacity(arr.len());
        for x in arr {
            let n = x
                .as_u64()
                .ok_or_else(|| "payload_bytes must be an array of u8 numbers".to_string())?;
            let b = u8::try_from(n)
                .map_err(|_| "payload_bytes element out of range (0..255)".to_string())?;
            out.push(b);
        }
        return Ok(PayloadSpec::Bytes(out));
    }
    if let Some(s) = params
        .get("payload_hex")
        .or_else(|| params.get("payload-hex"))
        .and_then(|v| v.as_str())
    {
        return Ok(PayloadSpec::Hex(s.to_string()));
    }
    if let Some(s) = params.get("payload").and_then(|v| v.as_str()) {
        return Ok(PayloadSpec::Text(s.to_string()));
    }
    Err("missing payload: provide one of payload (string) / payload_hex (hex string) / payload_bytes ([u8])".to_string())
}

fn publish_tx_request(
    sock_id: u64,
    payload: PayloadSpec,
    action_id: &str,
    user_id: &Option<String>,
    task_id: &Option<String>,
) -> Result<(), String> {
    let event_id = next_event_id();
    let mut obj = serde_json::Map::new();
    obj.insert(
        "sock_id".to_string(),
        serde_json::Value::Number(sock_id.into()),
    );
    obj.insert(
        "action_id".to_string(),
        serde_json::Value::String(action_id.to_string()),
    );
    obj.insert(
        "task_id".to_string(),
        task_id
            .as_ref()
            .map(|s| serde_json::Value::String(s.clone()))
            .unwrap_or(serde_json::Value::Null),
    );
    obj.insert(
        "user_id".to_string(),
        user_id
            .as_ref()
            .map(|s| serde_json::Value::String(s.clone()))
            .unwrap_or(serde_json::Value::Null),
    );
    match payload {
        PayloadSpec::Text(s) => {
            obj.insert("payload".to_string(), serde_json::Value::String(s));
        }
        PayloadSpec::Hex(s) => {
            obj.insert("payload_hex".to_string(), serde_json::Value::String(s));
        }
        PayloadSpec::Bytes(b) => {
            obj.insert(
                "payload_bytes".to_string(),
                serde_json::Value::Array(
                    b.into_iter().map(|x| serde_json::Value::from(x)).collect(),
                ),
            );
        }
    }
    let payload_json = serde_json::Value::Object(obj).to_string();
    ntx::scenario_eventbus::event_bus::publish(&ntx::scenario_eventbus::event_bus::Event {
        id: event_id,
        kind: "packet.tx-request".to_string(),
        user_id: user_id.clone(),
        task_id: task_id.clone(),
        action_id: Some(action_id.to_string()),
        payload: payload_json,
        correlation_id: None,
        timestamp_ms: now_ms(),
    })
    .map_err(|e| format!("publish tx-request failed: {e}"))?;
    Ok(())
}

fn poll_for_packet_rx(
    subscription_id: &str,
    want_user_id: &Option<String>,
    want_task_id: &Option<String>,
    want_action_id: &str,
    want_sock_id: u64,
    deadline_ms: Option<u64>,
    max_iters: u32,
) -> Result<Option<serde_json::Value>, String> {
    for _ in 0..max_iters {
        if let Some(dl) = deadline_ms {
            if now_ms() >= dl {
                return Ok(None);
            }
        }

        let events = ntx::scenario_eventbus::event_bus::poll_events(subscription_id, 64)
            .map_err(|e| format!("poll_events(packet.rx) failed: {e}"))?;
        if events.is_empty() {
            continue;
        }

        for ev in events {
            if ev.kind != "packet.rx" {
                continue;
            }

            // Best-effort match: action_id + user_id + (optional task_id) + payload.sock_id
            if ev.action_id.as_deref() != Some(want_action_id) {
                continue;
            }
            if want_user_id.is_some() && ev.user_id.as_ref() != want_user_id.as_ref() {
                continue;
            }
            if want_task_id.is_some() && ev.task_id.as_ref() != want_task_id.as_ref() {
                continue;
            }

            let p: serde_json::Value = serde_json::from_str(&ev.payload)
                .map_err(|e| format!("decode packet.rx payload json: {e}"))?;
            let sock_id = p
                .get("sock_id")
                .or_else(|| p.get("sock-id"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if sock_id != want_sock_id {
                continue;
            }

            return Ok(Some(p));
        }
    }
    Ok(None)
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
        let ctx_deadline_ms = ctx.as_ref().and_then(|c| c.deadline_ms);

        println!(
            "[actions-executor] execute action id={} call={} user={:?} task={:?}",
            action.id, action.call, user_id, task_id
        );

        match action.call.as_str() {
            // 对齐 packet-engine，但不直接调用 host：发布事件让 scheduler/host 侧处理。
            "udp.send-reply" | "udp.send" => {
                let params: serde_json::Value = serde_json::from_str(&action.params)
                    .map_err(|e| format!("parse params as json: {e}"))?;
                let sock_id = params
                    .get("socket_id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| "missing socket_id (u64)".to_string())?;
                let payload_spec = parse_payload_spec(&params)?;

                // 通过 eventbus 通知 scheduler/host 进行真正发包
                publish_tx_request(
                    sock_id,
                    payload_spec.clone(),
                    &action.id,
                    &user_id,
                    &task_id,
                )?;

                let exports = serde_json::json!({
                    "socket_id": sock_id,
                    "action_call": action.call,
                    "note": "tx delegated; rx (if any) is handled via scheduler packet.rx + workflow wait node",
                })
                .to_string();

                Ok(ntx::scenario_types::types::ActionOutcome {
                    status: ntx::scenario_types::types::OutcomeStatus::Success,
                    detail: Some(format!("{} delegated socket_id={}", action.call, sock_id)),
                    metrics: None,
                    exports: Some(exports),
                })
            }
            "udp.send-recv" => {
                let params: serde_json::Value = serde_json::from_str(&action.params)
                    .map_err(|e| format!("parse params as json: {e}"))?;
                let sock_id = params
                    .get("socket_id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| "missing socket_id (u64)".to_string())?;

                let payload_spec = parse_payload_spec(&params)?;
                let timeout_ms = parse_timeout_ms(&params);
                let deadline_ms = min_deadline_ms(now_ms(), ctx_deadline_ms, timeout_ms);

                // 1) Subscribe before tx to avoid missing a fast rx in the in-memory eventbus stub.
                let sub_id = ntx::scenario_eventbus::event_bus::subscribe("packet.rx")
                    .map_err(|e| format!("subscribe(packet.rx) failed: {e}"))?;

                // 2) Delegate tx to scheduler/host.
                if let Err(e) =
                    publish_tx_request(sock_id, payload_spec, &action.id, &user_id, &task_id)
                {
                    let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&sub_id);
                    return Err(e);
                }

                // 3) Poll until matching packet.rx arrives or timeout.
                let started = now_ms();
                let got = poll_for_packet_rx(
                    &sub_id,
                    &user_id,
                    &task_id,
                    &action.id,
                    sock_id,
                    deadline_ms,
                    10_000,
                );

                let _ = ntx::scenario_eventbus::event_bus::unsubscribe(&sub_id);

                let p = got?;
                match p {
                    Some(rx) => {
                        let latency = now_ms().saturating_sub(started);
                        let len = rx.get("len").and_then(|v| v.as_u64()).unwrap_or(0);
                        let exports = serde_json::json!({
                            "socket_id": sock_id,
                            "rx": rx,
                        })
                        .to_string();
                        Ok(ntx::scenario_types::types::ActionOutcome {
                            status: ntx::scenario_types::types::OutcomeStatus::Success,
                            detail: Some("udp.send-recv ok".to_string()),
                            metrics: Some(ntx::scenario_types::types::OutcomeMetrics {
                                latency_ms: Some(latency),
                                bytes_sent: None,
                                bytes_received: Some(len),
                                response_code: None,
                            }),
                            exports: Some(exports),
                        })
                    }
                    None => Ok(ntx::scenario_types::types::ActionOutcome {
                        status: ntx::scenario_types::types::OutcomeStatus::Timeout,
                        detail: Some("udp.send-recv timeout waiting for packet.rx".to_string()),
                        metrics: None,
                        exports: None,
                    }),
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_payload_text() {
        let v = serde_json::json!({"payload":"hello"});
        match parse_payload_spec(&v).unwrap() {
            PayloadSpec::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn parse_payload_hex() {
        let v = serde_json::json!({"payload_hex":"0a0b"});
        match parse_payload_spec(&v).unwrap() {
            PayloadSpec::Hex(s) => assert_eq!(s, "0a0b"),
            _ => panic!("expected hex"),
        }
    }

    #[test]
    fn parse_payload_bytes() {
        let v = serde_json::json!({"payload_bytes":[1,2,255]});
        match parse_payload_spec(&v).unwrap() {
            PayloadSpec::Bytes(b) => assert_eq!(b, vec![1, 2, 255]),
            _ => panic!("expected bytes"),
        }
    }
}
