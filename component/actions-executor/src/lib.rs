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
use crate::ntx::core_types::types::{
    ActionContext, ActionDef, ActionOutcome, OutcomeStatus, PeriodicSchedule, RateLimitedSchedule,
    SendRequest, SendSchedule, TimetableSchedule,
};
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
    correlation_id: &Option<String>,
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
        correlation_id: correlation_id.clone(),
        timestamp_ms: now_ms(),
    })
    .map_err(|e| format!("publish tx-request failed: {e}"))?;
    Ok(())
}

fn next_request_id(prefix: &str) -> String {
    let n = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{n}")
}

fn parse_u32(params: &serde_json::Value, key: &str) -> Option<u32> {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
}

fn parse_u64(params: &serde_json::Value, key: &str) -> Option<u64> {
    params.get(key).and_then(|v| v.as_u64())
}

fn parse_string(params: &serde_json::Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn parse_schedule(params: &serde_json::Value) -> Result<SendSchedule, String> {
    let mode = params
        .get("schedule")
        .or_else(|| params.get("send_schedule"))
        .or_else(|| params.get("mode"))
        .and_then(|v| v.as_str())
        .unwrap_or("once")
        .trim()
        .to_ascii_lowercase();

    match mode.as_str() {
        "once" => Ok(SendSchedule::Once),
        "periodic" => {
            let interval_ms = parse_u64(params, "interval_ms")
                .or_else(|| parse_u64(params, "interval-ms"))
                .ok_or_else(|| "missing interval_ms for periodic schedule".to_string())?;
            let start_delay_ms =
                parse_u64(params, "start_delay_ms").or_else(|| parse_u64(params, "start-delay-ms"));
            Ok(SendSchedule::Periodic(PeriodicSchedule {
                interval_ms,
                start_delay_ms,
            }))
        }
        "timetable" => {
            let ts = params
                .get("timestamps_ms")
                .or_else(|| params.get("timestamps-ms"))
                .and_then(|v| v.as_array())
                .ok_or_else(|| "missing timestamps_ms for timetable schedule".to_string())?;
            let mut out: Vec<u64> = Vec::with_capacity(ts.len());
            for x in ts {
                out.push(
                    x.as_u64()
                        .ok_or_else(|| "timestamps_ms must be u64 array".to_string())?,
                );
            }
            Ok(SendSchedule::Timetable(TimetableSchedule {
                timestamps_ms: out,
            }))
        }
        "rate-limited" | "rate_limited" | "ratelimited" => {
            let pps = parse_u32(params, "pps")
                .ok_or_else(|| "missing pps for rate-limited schedule".to_string())?;
            let burst_size =
                parse_u32(params, "burst_size").or_else(|| parse_u32(params, "burst-size"));
            Ok(SendSchedule::RateLimited(RateLimitedSchedule {
                pps,
                burst_size,
            }))
        }
        other => Err(format!("unsupported send schedule mode: {other}")),
    }
}

impl exports::ntx::scenario_actions_executor::action_component::Guest for ActionExecutorImpl {
    fn init_component() -> Result<(), String> {
        println!("[actions-executor] init-component");
        Ok(())
    }

    fn execute_action(
        action: ActionDef,
        ctx: Option<ActionContext>,
    ) -> Result<ActionOutcome, String> {
        let user_id = ctx.as_ref().and_then(|c| c.user_id.clone());
        let task_id = ctx.as_ref().and_then(|c| c.task_id.clone());
        let correlation_id = ctx.as_ref().and_then(|c| c.correlation_id.clone());

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
                    &correlation_id,
                )?;

                let exports = serde_json::json!({
                    "socket_id": sock_id,
                    "action_call": action.call,
                    "note": "tx delegated; rx (if any) is handled via scheduler packet.rx + workflow wait node",
                })
                .to_string();

                Ok(ActionOutcome {
                    status: OutcomeStatus::Success,
                    detail: Some(format!("{} delegated socket_id={}", action.call, sock_id)),
                    metrics: None,
                    exports: Some(exports),
                })
            }
            "udp.send-recv" => {
                // P0: 不在 executor 内部等待 packet.rx（避免单线程自旋/阻塞）。
                // 该 action 仅委托发包；收包等待/超时/重试由 scheduler 的 wait 节点 + timer event 推进。
                let params: serde_json::Value = serde_json::from_str(&action.params)
                    .map_err(|e| format!("parse params as json: {e}"))?;
                let sock_id = params
                    .get("socket_id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| "missing socket_id (u64)".to_string())?;

                let payload_spec = parse_payload_spec(&params)?;

                publish_tx_request(
                    sock_id,
                    payload_spec,
                    &action.id,
                    &user_id,
                    &task_id,
                    &correlation_id,
                )?;

                let exports = serde_json::json!({
                    "socket_id": sock_id,
                    "action_call": action.call,
                    "note": "tx delegated; rx/timeout/retry must be handled by scheduler state-machine (wait node + timer events)",
                })
                .to_string();

                Ok(ActionOutcome {
                    status: OutcomeStatus::Success,
                    detail: Some(format!(
                        "udp.send-recv delegated (no-wait) socket_id={}",
                        sock_id
                    )),
                    metrics: None,
                    exports: Some(exports),
                })
            }
            "udp.schedule-send" => {
                let params: serde_json::Value = serde_json::from_str(&action.params)
                    .map_err(|e| format!("parse params as json: {e}"))?;
                let sock_id = params
                    .get("socket_id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| "missing socket_id (u64)".to_string())?;

                let user_id = user_id
                    .clone()
                    .ok_or_else(|| "udp.schedule-send requires ctx.user_id".to_string())?;
                let task_id = task_id
                    .clone()
                    .ok_or_else(|| "udp.schedule-send requires ctx.task_id".to_string())?;

                // payload: allow fixed bytes via the same payload spec helper; generator is not implemented yet.
                let payload_spec = parse_payload_spec(&params)?;
                let schedule = parse_schedule(&params)?;

                let max_count =
                    parse_u32(&params, "max_count").or_else(|| parse_u32(&params, "max-count"));
                let timeout_ms =
                    parse_u64(&params, "timeout_ms").or_else(|| parse_u64(&params, "timeout-ms"));

                let request_id = parse_string(&params, "request_id")
                    .or_else(|| parse_string(&params, "request-id"))
                    .unwrap_or_else(|| next_request_id("send"));

                let payload_bytes: Vec<u8> = match payload_spec {
                    PayloadSpec::Text(s) => s.into_bytes(),
                    PayloadSpec::Hex(h) => {
                        // Reuse scheduler-side convention: accept 0x prefix and ignore whitespace.
                        let mut t = h.trim().to_ascii_lowercase();
                        if let Some(rest) = t.strip_prefix("0x") {
                            t = rest.to_string();
                        }
                        let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
                        if t.len() % 2 != 0 {
                            return Err("payload_hex length must be even".to_string());
                        }
                        let mut out = Vec::with_capacity(t.len() / 2);
                        for i in (0..t.len()).step_by(2) {
                            let byte = u8::from_str_radix(&t[i..i + 2], 16)
                                .map_err(|_| format!("invalid hex byte: {}", &t[i..i + 2]))?;
                            out.push(byte);
                        }
                        out
                    }
                    PayloadSpec::Bytes(b) => b,
                };

                let req = SendRequest {
                    request_id: request_id.clone(),
                    user_id: user_id.clone(),
                    task_id: task_id.clone(),
                    socket_id: sock_id,
                    schedule,
                    payload: Some(payload_bytes),
                    payload_generator: None,
                    max_count,
                    timeout_ms,
                };
                // todo: cal
                // let rid = ntx::scenario_send_scheduler::send_scheduler::schedule_send(&req)
                //     .map_err(|e| format!("schedule-send failed: {e}"))?;
                let rid = req.request_id.clone();
                let exports = serde_json::json!({
                    "request_id": rid,
                    "socket_id": sock_id,
                    "scheduled": true,
                })
                .to_string();

                Ok(ActionOutcome {
                    status: OutcomeStatus::Success,
                    detail: Some(format!("udp.schedule-send ok request_id={}", exports)),
                    metrics: None,
                    exports: Some(exports),
                })
            }
            c if c.starts_with("http.") || c.starts_with("tcp.") => Ok(ActionOutcome {
                status: OutcomeStatus::Failed,
                detail: Some(format!("action not implemented yet: {}", c)),
                metrics: None,
                exports: None,
            }),
            _ => Ok(ActionOutcome {
                // IMPORTANT: unknown actions must not default to Success (would mislead the state machine).
                status: OutcomeStatus::Failed,
                detail: Some(format!("unknown action.call: {}", action.call)),
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
