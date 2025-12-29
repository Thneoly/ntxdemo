//! Shared helpers for Scenario components ("frame"/SDK).
//!
//! Goal: make new components easy to bootstrap by reusing:
//! - payload spec parsing (payload/payload_hex/payload_bytes)
//! - send schedule parsing (once/periodic/timetable/rate-limited)
//! - common event publishing for tx + send scheduling
//!
//! This crate is intentionally small and dependency-light so it can be linked
//! into wasm32-wasip2 component crates.

use once_cell::sync::Lazy;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Small JSON param getter helpers.
///
/// These are intentionally tiny (no extra deps) and designed for component
/// action param parsing where keys often have legacy aliases like `foo_bar`
/// and `foo-bar`.
pub mod params {
    /// Get a value by trying multiple possible keys in order.
    pub fn get<'a>(params: &'a serde_json::Value, keys: &[&str]) -> Option<&'a serde_json::Value> {
        for k in keys {
            if let Some(v) = params.get(*k) {
                return Some(v);
            }
        }
        None
    }

    pub fn opt_u64(params: &serde_json::Value, keys: &[&str]) -> Option<u64> {
        get(params, keys).and_then(|v| v.as_u64())
    }

    pub fn opt_u32(params: &serde_json::Value, keys: &[&str]) -> Option<u32> {
        get(params, keys)
            .and_then(|v| v.as_u64())
            .and_then(|n| u32::try_from(n).ok())
    }

    pub fn opt_string(params: &serde_json::Value, keys: &[&str]) -> Option<String> {
        get(params, keys)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn req_u64(params: &serde_json::Value, keys: &[&str]) -> Result<u64, String> {
        let joined = keys.join("|");
        let v = get(params, keys).ok_or_else(|| format!("missing {} (u64)", joined))?;
        v.as_u64()
            .ok_or_else(|| format!("{} must be a u64", joined))
    }

    pub fn req_u32(params: &serde_json::Value, keys: &[&str]) -> Result<u32, String> {
        let joined = keys.join("|");
        let v = get(params, keys).ok_or_else(|| format!("missing {} (u32)", joined))?;
        let n = v
            .as_u64()
            .ok_or_else(|| format!("{} must be a u32", joined))?;
        u32::try_from(n).map_err(|_| format!("{} out of range for u32", joined))
    }

    pub fn req_string(params: &serde_json::Value, keys: &[&str]) -> Result<String, String> {
        let joined = keys.join("|");
        let v = get(params, keys).ok_or_else(|| format!("missing {} (string)", joined))?;
        let s = v
            .as_str()
            .ok_or_else(|| format!("{} must be a string", joined))?;
        Ok(s.to_string())
    }
}

/// Normalize JSON object keys so typed deserialization can be more stable across
/// legacy alias styles.
///
/// Current behavior:
/// - For objects, any key containing `-` will also be inserted with `-` replaced
///   by `_` if that alias key doesn't already exist.
/// - Nested objects/arrays are processed recursively.
///
/// This makes typed structs like `max_count` accept input `max-count` without
/// having to annotate every field.
pub fn normalize_param_keys(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();

            // First pass: normalized copy of all children.
            for (k, vv) in map.iter() {
                out.insert(k.clone(), normalize_param_keys(vv));
            }

            // Second pass: add kebab->snake aliases.
            let keys: Vec<String> = out.keys().cloned().collect();
            for k in keys {
                if k.contains('-') {
                    let alt = k.replace('-', "_");
                    if !out.contains_key(&alt) {
                        if let Some(vv) = out.get(&k).cloned() {
                            out.insert(alt, vv);
                        }
                    }
                }
            }

            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(normalize_param_keys).collect())
        }
        other => other.clone(),
    }
}

/// Parse typed params from an [`ActionRequest`].
///
/// This is the "strong framework" path: handlers can define a typed struct and
/// let the framework decode it.
///
/// The input JSON is first normalized via [`normalize_param_keys`] so common
/// alias styles work out of the box.
pub fn parse_params<T>(req: &ActionRequest) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    let normalized = normalize_param_keys(&req.params_json);
    serde_json::from_value(normalized).map_err(|e| format!("parse params for {}: {e}", req.call))
}

/// Require `ctx.user_id` and `ctx.task_id` from an [`ActionRuntime`].
///
/// This is a tiny helper to remove repetitive boilerplate in handlers.
///
/// Returns `(user_id, task_id)`.
#[macro_export]
macro_rules! require_user_task {
    ($rt:expr, $req:expr) => {{
        let user_id = ($rt)
            .ctx
            .user_id
            .clone()
            .ok_or_else(|| format!("{} requires ctx.user_id", ($req).call))?;
        let task_id = ($rt)
            .ctx
            .task_id
            .clone()
            .ok_or_else(|| format!("{} requires ctx.task_id", ($req).call))?;
        (user_id, task_id)
    }};
}

static EVENT_SEQ: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(1));

static REQUEST_SEQ: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(1));

fn next_event_id(prefix: &str) -> String {
    let n = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{n}")
}

/// Generate a monotonic request id (`{prefix}-{n}`), suitable for correlating
/// scheduler-managed requests (e.g. send schedule requests).
///
/// This is intentionally *not* a UUID so it stays lightweight for wasm builds.
pub fn next_request_id(prefix: &str) -> String {
    let n = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{n}")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Payload specification as accepted by actions and tx-requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadSpec {
    Text(String),
    Hex(String),
    Bytes(Vec<u8>),
}

/// Parse payload from params json.
///
/// Accepts:
/// - `payload_bytes: [u8]`
/// - `payload_hex` or `payload-hex`: hex string (may include `0x` prefix/whitespace)
/// - `payload`: string
pub fn parse_payload_spec(params: &serde_json::Value) -> Result<PayloadSpec, String> {
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

/// Decode a `PayloadSpec` into raw bytes. Hex parsing follows the same behavior
/// currently used by the scheduler: accept `0x` prefix and ignore whitespace.
pub fn payload_to_bytes(spec: PayloadSpec) -> Result<Vec<u8>, String> {
    match spec {
        PayloadSpec::Text(s) => Ok(s.into_bytes()),
        PayloadSpec::Bytes(b) => Ok(b),
        PayloadSpec::Hex(h) => decode_hex_bytes(&h),
    }
}

pub fn decode_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    let mut t = s.trim().to_ascii_lowercase();
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
    Ok(out)
}

/// Parse `SendSchedule` from params JSON.
///
/// This is a generic helper that doesn't depend on core-types, so it can be used
/// by components that have different schedule types.
pub fn parse_schedule_like(params: &serde_json::Value) -> Result<ScheduleLike, String> {
    let mode = params
        .get("schedule")
        .or_else(|| params.get("send_schedule"))
        .or_else(|| params.get("mode"))
        .and_then(|v| v.as_str())
        .unwrap_or("once")
        .trim()
        .to_ascii_lowercase();

    match mode.as_str() {
        "once" => Ok(ScheduleLike::Once),
        "periodic" => {
            let interval_ms = params
                .get("interval_ms")
                .or_else(|| params.get("interval-ms"))
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "missing interval_ms for periodic schedule".to_string())?;
            let start_delay_ms = params
                .get("start_delay_ms")
                .or_else(|| params.get("start-delay-ms"))
                .and_then(|v| v.as_u64());
            Ok(ScheduleLike::Periodic {
                interval_ms,
                start_delay_ms,
            })
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
            Ok(ScheduleLike::Timetable { timestamps_ms: out })
        }
        "rate-limited" | "rate_limited" | "ratelimited" => {
            let pps = params
                .get("pps")
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| "missing pps for rate-limited schedule".to_string())?;
            let burst_size = params
                .get("burst_size")
                .or_else(|| params.get("burst-size"))
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok());
            Ok(ScheduleLike::RateLimited { pps, burst_size })
        }
        other => Err(format!("unsupported send schedule mode: {other}")),
    }
}

/// Small schedule enum that can be converted into component-specific schedule types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleLike {
    Once,
    Periodic {
        interval_ms: u64,
        start_delay_ms: Option<u64>,
    },
    Timetable {
        timestamps_ms: Vec<u64>,
    },
    RateLimited {
        pps: u32,
        burst_size: Option<u32>,
    },
}

/// Publish `packet.tx-request`.
///
/// This is generic over the event type so it can work in wasm component crates
/// (which have a WIT-generated `Event` type) and in host crates.
pub fn publish_packet_tx_request<Ev, PublishFn>(
    sock_id: u64,
    payload: PayloadSpec,
    action_id: &str,
    user_id: &Option<String>,
    task_id: &Option<String>,
    correlation_id: &Option<String>,
    mut publish: PublishFn,
    make_event: impl FnOnce(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        u64,
    ) -> Ev,
) -> Result<(), String>
where
    PublishFn: FnMut(&Ev) -> Result<(), String>,
{
    let mut obj = serde_json::Map::new();
    obj.insert(
        "sock_id".to_string(),
        serde_json::Value::Number(sock_id.into()),
    );
    obj.insert(
        "action_id".to_string(),
        serde_json::Value::String(action_id.to_string()),
    );
    obj.insert("task_id".to_string(), json!(task_id));
    obj.insert("user_id".to_string(), json!(user_id));

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
                serde_json::Value::Array(b.into_iter().map(serde_json::Value::from).collect()),
            );
        }
    }

    let payload_json = serde_json::Value::Object(obj).to_string();

    let ev = make_event(
        next_event_id("ae"),
        "packet.tx-request".to_string(),
        user_id.clone(),
        task_id.clone(),
        Some(action_id.to_string()),
        payload_json,
        correlation_id.clone(),
        now_ms(),
    );

    publish(&ev).map_err(|e| format!("publish tx-request failed: {e}"))?;
    Ok(())
}

// ---------------------------
// Framework layer (traits/macros)
// ---------------------------

/// Minimal, component-agnostic view of an action request.
///
/// Each component can adapt its WIT-generated `ActionDef` into this.
#[derive(Debug, Clone)]
pub struct ActionRequest {
    pub id: String,
    pub call: String,
    pub params_json: serde_json::Value,
}

/// Minimal, component-agnostic context.
#[derive(Debug, Clone, Default)]
pub struct ActionCtx {
    pub user_id: Option<String>,
    pub task_id: Option<String>,
    pub correlation_id: Option<String>,
}

/// Outcome status used by the framework.
///
/// Component adapters convert this into their WIT `OutcomeStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameworkStatus {
    Success,
    Failed,
}

/// Component-agnostic outcome.
#[derive(Debug, Clone)]
pub struct FrameworkOutcome {
    pub status: FrameworkStatus,
    pub detail: Option<String>,
    pub exports_json: Option<String>,
}

impl FrameworkOutcome {
    pub fn success(detail: impl Into<String>) -> Self {
        Self {
            status: FrameworkStatus::Success,
            detail: Some(detail.into()),
            exports_json: None,
        }
    }

    pub fn failed(detail: impl Into<String>) -> Self {
        Self {
            status: FrameworkStatus::Failed,
            detail: Some(detail.into()),
            exports_json: None,
        }
    }

    pub fn with_exports_json(mut self, exports_json: String) -> Self {
        self.exports_json = Some(exports_json);
        self
    }
}

/// Adapter trait: lets the framework publish events without depending on WIT.
pub trait EventBusAdapter {
    type Event;

    fn make_event(
        &self,
        id: String,
        kind: String,
        user_id: Option<String>,
        task_id: Option<String>,
        action_id: Option<String>,
        payload: String,
        correlation_id: Option<String>,
        timestamp_ms: u64,
    ) -> Self::Event;

    fn publish(&self, ev: &Self::Event) -> Result<(), String>;
}

/// Runtime passed to action handlers; provides standard helpers.
pub struct ActionRuntime<'a, Bus: EventBusAdapter> {
    pub bus: &'a Bus,
    pub ctx: ActionCtx,
}

impl<'a, Bus: EventBusAdapter> ActionRuntime<'a, Bus> {
    pub fn publish_tx_request(
        &self,
        sock_id: u64,
        payload: PayloadSpec,
        action_id: &str,
    ) -> Result<(), String> {
        publish_packet_tx_request(
            sock_id,
            payload,
            action_id,
            &self.ctx.user_id,
            &self.ctx.task_id,
            &self.ctx.correlation_id,
            |ev| self.bus.publish(ev),
            |id, kind, user_id, task_id, action_id, payload, correlation_id, timestamp_ms| {
                self.bus.make_event(
                    id,
                    kind,
                    user_id,
                    task_id,
                    action_id,
                    payload,
                    correlation_id,
                    timestamp_ms,
                )
            },
        )
    }

    /// Helper to standardize an "unknown action" failure.
    pub fn unknown_action(&self, call: &str) -> FrameworkOutcome {
        FrameworkOutcome::failed(format!("unknown action.call: {call}"))
    }
}

/// Trait implemented by a component's action module.
///
/// This is the main "framework" extension point.
pub trait ActionModule<Bus: EventBusAdapter> {
    fn handle(
        &self,
        rt: &ActionRuntime<'_, Bus>,
        req: &ActionRequest,
    ) -> Result<FrameworkOutcome, String>;
}

/// Dispatch macro: enforces a consistent match-based routing structure.
///
/// Usage:
/// ```ignore
/// ntx_action_sdk::declare_actions!(req.call.as_str(), {
///   "udp.send" => |rt, req| { ... },
///   "udp.send-reply" => |rt, req| { ... },
/// });
/// ```
#[macro_export]
macro_rules! declare_actions {
    ($call:expr, { $($pat:pat => $handler:expr),+ $(,)? }) => {{
        match $call {
            $($pat => $handler),+,
            other => {
                let other = other;
                Err(format!("unknown action.call: {}", other))
            }
        }
    }};
}

/// Stronger declarative routing macro.
///
/// It generates a dispatcher expression that you can call with:
/// `(rt, req) -> Result<FrameworkOutcome, String>`.
///
/// Features:
/// - exact match: `"udp.send" => handler`
/// - aliases: `["udp.send", "udp.send-reply"] => handler`
/// - prefix match: `prefix "http." => handler`
/// - required fallback: `_ => handler`
///
/// Handler can be a function item (`fn`) or a closure expression.
/// The handler signature should be:
/// `fn(&ActionRuntime<Bus>, &ActionRequest) -> Result<FrameworkOutcome, String>`
///
/// Example:
/// ```ignore
/// You can also use the *inline* form (most reliable):
/// ```ignore
/// ntx_action_sdk::routes!(rt, req, {
///   alias ["udp.send", "udp.send-reply"] => handle_udp_send,
///   prefix "http." => handle_http,
///   _ => |rt, req| Ok(rt.unknown_action(&req.call)),
/// })?;
/// ```
/// ```
#[macro_export]
macro_rules! routes {
    // Inline dispatcher form.
    (
        $rt:expr,
        $req:expr,
        {
            $($body:tt)*
        }
    ) => {{
        $crate::routes!(@inline $rt, $req, $($body)*)
    }};

    // alias
    (@inline $rt:expr, $req:expr, alias [ $($lit:literal),+ $(,)? ] => $h:expr, $($rest:tt)*) => {{
        if false $(|| ($req).call == $lit)+ {
            ($h)($rt, $req)
        } else {
            $crate::routes!(@inline $rt, $req, $($rest)*)
        }
    }};

    // exact
    (@inline $rt:expr, $req:expr, $lit:literal => $h:expr, $($rest:tt)*) => {{
        if ($req).call == $lit {
            ($h)($rt, $req)
        } else {
            $crate::routes!(@inline $rt, $req, $($rest)*)
        }
    }};

    // fallback (must be last)
    (@inline $rt:expr, $req:expr, _ => $h:expr $(,)?) => {{
        ($h)($rt, $req)
    }};

    // prefix
    (@inline $rt:expr, $req:expr, prefix $p:literal => $h:expr, $($rest:tt)*) => {{
        if ($req).call.starts_with($p) {
            ($h)($rt, $req)
        } else {
            $crate::routes!(@inline $rt, $req, $($rest)*)
        }
    }};

    // Apply rules in order.
    (@apply $rt:ident, $req:ident,) => {};

    // exact
    (@apply $rt:ident, $req:ident, exact($lit:literal, $h:expr); $($rest:tt)*) => {{
        if $req.call == $lit {
            return ($h)($rt, $req);
        }
        $crate::routes!(@apply $rt, $req, $($rest)*);
    }};

    // alias
    (@apply $rt:ident, $req:ident, alias([ $($lit:literal),+ $(,)? ], $h:expr); $($rest:tt)*) => {{
        if false $(|| $req.call == $lit)+ {
            return ($h)($rt, $req);
        }
        $crate::routes!(@apply $rt, $req, $($rest)*);
    }};

    // prefix
    (@apply $rt:ident, $req:ident, prefix($p:literal, $h:expr); $($rest:tt)*) => {{
        if $req.call.starts_with($p) {
            return ($h)($rt, $req);
        }
        $crate::routes!(@apply $rt, $req, $($rest)*);
    }};

    // Ignore fallback if not in the last position (we require it last via outer rule).
    (@apply $rt:ident, $req:ident, fallback($h:expr); $($rest:tt)*) => {{
        let _ = $h;
        $crate::routes!(@apply $rt, $req, $($rest)*);
    }};
}

/// Helper macro to build `exports` JSON strings consistently.
#[macro_export]
macro_rules! exports_json {
    ($($json:tt)+) => {{
        serde_json::json!($($json)+).to_string()
    }};
}

/// Publish `send.schedule-request` event with the same JSON payload schema used
/// by the scheduler.
pub fn publish_send_schedule_request<Ev, PublishFn>(
    req: &serde_json::Value,
    action_id: &str,
    correlation_id: &Option<String>,
    user_id: &str,
    task_id: &str,
    mut publish: PublishFn,
    make_event: impl FnOnce(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        u64,
    ) -> Ev,
) -> Result<(), String>
where
    PublishFn: FnMut(&Ev) -> Result<(), String>,
{
    let ev = make_event(
        format!(
            "send-req-{}",
            req.get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        ),
        "send.schedule-request".to_string(),
        Some(user_id.to_string()),
        Some(task_id.to_string()),
        Some(action_id.to_string()),
        req.to_string(),
        correlation_id.clone(),
        now_ms(),
    );

    publish(&ev).map_err(|e| format!("publish send.schedule-request failed: {e}"))?;
    Ok(())
}

/// Build the JSON payload for `send.schedule-request` with the schema expected
/// by the scheduler.
///
/// This helper stays component-agnostic by taking primitives instead of WIT
/// types.
///
/// Notes:
/// - `payload_bytes` becomes `payload_bytes: [u8]` when present.
/// - `payload_generator` is passed through as-is when present.
pub fn build_send_schedule_request_payload(
    request_id: &str,
    user_id: &str,
    task_id: &str,
    socket_id: u64,
    max_count: Option<u32>,
    timeout_ms: Option<u64>,
    schedule: &ScheduleLike,
    payload_bytes: Option<&[u8]>,
    payload_generator: Option<&serde_json::Value>,
) -> serde_json::Value {
    let schedule_json = match schedule {
        ScheduleLike::Once => json!({"mode":"once"}),
        ScheduleLike::Periodic {
            interval_ms,
            start_delay_ms,
        } => json!({
            "mode":"periodic",
            "interval_ms": interval_ms,
            "start_delay_ms": start_delay_ms,
        }),
        ScheduleLike::Timetable { timestamps_ms } => json!({
            "mode":"timetable",
            "timestamps_ms": timestamps_ms,
        }),
        ScheduleLike::RateLimited { pps, burst_size } => json!({
            "mode":"rate-limited",
            "pps": pps,
            "burst_size": burst_size,
        }),
    };

    let mut obj = serde_json::Map::new();
    obj.insert("request_id".to_string(), json!(request_id));
    obj.insert("user_id".to_string(), json!(user_id));
    obj.insert("task_id".to_string(), json!(task_id));
    obj.insert("socket_id".to_string(), json!(socket_id));
    obj.insert(
        "max_count".to_string(),
        max_count
            .map(|v| json!(v))
            .unwrap_or(serde_json::Value::Null),
    );
    obj.insert(
        "timeout_ms".to_string(),
        timeout_ms
            .map(|v| json!(v))
            .unwrap_or(serde_json::Value::Null),
    );
    obj.insert("schedule".to_string(), schedule_json);

    if let Some(payload) = payload_bytes {
        obj.insert("payload_bytes".to_string(), json!(payload));
    }

    if let Some(gen) = payload_generator {
        obj.insert("payload_generator".to_string(), gen.clone());
    }

    serde_json::Value::Object(obj)
}

// ---------------------------
// Optional adapters (feature-gated)
// ---------------------------

/// Generate adapters for a component's WIT-generated schedule/request types.
///
/// Why a macro?
/// - `SendSchedule`/`SendRequest` types are generated by `wit-bindgen` *per world*.
/// - Even if they look identical, types from `core-types` world and
///   `actions-executor` world are distinct, so a normal Rust function can't take
///   both.
///
/// This macro lets each component bind the SDK helpers to *its own* generated
/// types by specifying the module path containing `SendSchedule`, `SendRequest`,
/// `PeriodicSchedule`, `RateLimitedSchedule`, and `TimetableSchedule`.
///
/// Example (in a component crate):
/// ```ignore
/// ntx_action_sdk::define_core_types_adapters!(types);
/// ```
#[cfg(feature = "core-types-adapter")]
#[macro_export]
macro_rules! define_core_types_adapters {
    ($types_mod:ident) => {
        pub fn schedule_like_from_core(
            schedule: &$types_mod::SendSchedule,
        ) -> $crate::ScheduleLike {
            match schedule {
                $types_mod::SendSchedule::Once => $crate::ScheduleLike::Once,
                $types_mod::SendSchedule::Periodic(p) => $crate::ScheduleLike::Periodic {
                    interval_ms: p.interval_ms,
                    start_delay_ms: p.start_delay_ms,
                },
                $types_mod::SendSchedule::Timetable(t) => $crate::ScheduleLike::Timetable {
                    timestamps_ms: t.timestamps_ms.clone(),
                },
                $types_mod::SendSchedule::RateLimited(r) => $crate::ScheduleLike::RateLimited {
                    pps: r.pps,
                    burst_size: r.burst_size,
                },
            }
        }

        pub fn schedule_like_to_core(schedule: &$crate::ScheduleLike) -> $types_mod::SendSchedule {
            match schedule {
                $crate::ScheduleLike::Once => $types_mod::SendSchedule::Once,
                $crate::ScheduleLike::Periodic {
                    interval_ms,
                    start_delay_ms,
                } => $types_mod::SendSchedule::Periodic($types_mod::PeriodicSchedule {
                    interval_ms: *interval_ms,
                    start_delay_ms: *start_delay_ms,
                }),
                $crate::ScheduleLike::Timetable { timestamps_ms } => {
                    $types_mod::SendSchedule::Timetable($types_mod::TimetableSchedule {
                        timestamps_ms: timestamps_ms.clone(),
                    })
                }
                $crate::ScheduleLike::RateLimited { pps, burst_size } => {
                    $types_mod::SendSchedule::RateLimited($types_mod::RateLimitedSchedule {
                        pps: *pps,
                        burst_size: *burst_size,
                    })
                }
            }
        }

        pub fn parse_schedule_core(
            params: &serde_json::Value,
        ) -> Result<$types_mod::SendSchedule, String> {
            let like = $crate::parse_schedule_like(params)?;
            Ok(schedule_like_to_core(&like))
        }

        pub fn publish_send_schedule_request_from_core<Ev, PublishFn>(
            req: &$types_mod::SendRequest,
            action_id: &str,
            correlation_id: &Option<String>,
            mut publish: PublishFn,
            make_event: impl FnOnce(
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
                Option<String>,
                u64,
            ) -> Ev,
        ) -> Result<(), String>
        where
            PublishFn: FnMut(&Ev) -> Result<(), String>,
        {
            let schedule_like = schedule_like_from_core(&req.schedule);

            let payload_json = $crate::build_send_schedule_request_payload(
                &req.request_id,
                &req.user_id,
                &req.task_id,
                req.socket_id,
                req.max_count,
                req.timeout_ms,
                &schedule_like,
                req.payload.as_deref(),
                None,
            );

            $crate::publish_send_schedule_request(
                &payload_json,
                action_id,
                correlation_id,
                &req.user_id,
                &req.task_id,
                |ev| publish(ev),
                make_event,
            )
        }
    };
}

/// Generate the remaining WIT glue for components that use the scheduler
/// `send.schedule-request` flow.
///
/// This macro bundles:
/// - `define_core_types_adapters!(types_mod)` (feature-gated)
/// - `parse_schedule(params) -> SendSchedule`
/// - `publish_send_schedule_request(req, action_id, correlation_id)`
///
/// The generated helpers intentionally live in the *component crate* (because
/// WIT-generated types differ per world).
///
/// Example:
/// ```ignore
/// use crate::ntx::core_types::types;
/// ntx_action_sdk::define_wit_scheduler_send_glue!(
///   types_mod = types,
///   event_ty = crate::ntx::scenario_eventbus::event_bus::Event,
///   publish_fn = crate::ntx::scenario_eventbus::event_bus::publish,
/// );
/// ```
#[macro_export]
macro_rules! define_wit_scheduler_send_glue {
    (
        types_mod = $types_mod:ident,
        event_ty = $event_ty:path,
        publish_fn = $publish_fn:path
        $(,)?
    ) => {
        // Bind schedule/request adapters to this component's WIT-generated types.
        $crate::define_core_types_adapters!($types_mod);

        fn publish_send_schedule_request(
            req: &$types_mod::SendRequest,
            action_id: &str,
            correlation_id: &Option<String>,
        ) -> Result<(), String> {
            // Keep existing behavior: payload_generator is not implemented by default.
            if req.payload_generator.is_some() {
                println!("[component] warn: payload_generator not implemented; ignoring");
            }

            publish_send_schedule_request_from_core(
                req,
                action_id,
                correlation_id,
                |ev| ($publish_fn)(ev).map_err(|e| e.to_string()),
                |id, kind, user_id, task_id, action_id, payload, correlation_id, timestamp_ms| {
                    $event_ty {
                        id,
                        kind,
                        user_id,
                        task_id,
                        action_id,
                        payload,
                        correlation_id,
                        timestamp_ms,
                    }
                },
            )
        }

        fn parse_schedule(params: &serde_json::Value) -> Result<$types_mod::SendSchedule, String> {
            parse_schedule_core(params)
        }
    };
}

/// Protocol-agnostic one-shot macro to generate a schedule-send handler.
///
/// The SDK should not be UDP-specific. This macro keeps the common skeleton
/// and lets the component inject:
/// - how to parse typed params
/// - how to build the request (WIT type)
/// - how to build exports
/// - how to format the success message
///
/// Preconditions:
/// - The component has already invoked [`define_wit_scheduler_send_glue!`], which provides:
///   `parse_schedule` and `publish_send_schedule_request` in scope.
#[macro_export]
macro_rules! define_schedule_send_handler {
    (
        fn $fn_name:ident,
        bus = $bus_ty:ty,
        // Parse request-specific params from ActionRequest.
        parse_params = $parse_params:expr,
        // Build the WIT request to be published.
        build_request = $build_request:expr,
        // Build exports JSON (string) for the outcome.
        build_exports = $build_exports:expr,
        // Format the FrameworkOutcome detail string.
        success_detail = $success_detail:expr
        $(,)?
    ) => {
        fn $fn_name(
            rt: &$crate::ActionRuntime<'_, $bus_ty>,
            req: &$crate::ActionRequest,
        ) -> Result<$crate::FrameworkOutcome, String> {
            // 1) Parse typed params (component decides the schema)
            let parsed = ($parse_params)(req)?;

            // 2) Required ctx values (scheduler publish needs a user_id/task_id conventionally)
            let user_id = rt
                .ctx
                .user_id
                .clone()
                .ok_or_else(|| format!("{} requires ctx.user_id", req.call))?;
            let task_id = rt
                .ctx
                .task_id
                .clone()
                .ok_or_else(|| format!("{} requires ctx.task_id", req.call))?;

            // 3) Parse payload + schedule (shared)
            let payload_spec = $crate::parse_payload_spec(&req.params_json)?;
            let schedule = parse_schedule(&req.params_json)?;
            let payload_bytes: Vec<u8> = $crate::payload_to_bytes(payload_spec)?;

            // 4) Build request + publish (component decides the concrete WIT type)
            let send_req = ($build_request)(parsed, &user_id, &task_id, schedule, payload_bytes)?;

            publish_send_schedule_request(&send_req, &req.id, &rt.ctx.correlation_id)?;

            // 5) Exports + outcome
            let exports = ($build_exports)(&send_req);
            Ok(
                $crate::FrameworkOutcome::success(($success_detail)(&exports))
                    .with_exports_json(exports),
            )
        }
    };
}

/// Generate a `WITEventBus` adapter type for the framework.
///
/// This removes boilerplate like:
/// - defining `struct WITEventBus;`
/// - implementing [`EventBusAdapter`] by wiring `make_event` fields
/// - calling the WIT publish function
///
/// Usage (in a component crate):
/// ```ignore
/// ntx_action_sdk::define_wit_event_bus!(
///   WITEventBus,
///   ntx::scenario_eventbus::event_bus::Event,
///   ntx::scenario_eventbus::event_bus::publish
/// );
/// ```
#[macro_export]
macro_rules! define_wit_event_bus {
    ($bus_ty:ident, $event_ty:path, $publish_fn:path) => {
        pub struct $bus_ty;

        impl $crate::EventBusAdapter for $bus_ty {
            type Event = $event_ty;

            fn make_event(
                &self,
                id: String,
                kind: String,
                user_id: Option<String>,
                task_id: Option<String>,
                action_id: Option<String>,
                payload: String,
                correlation_id: Option<String>,
                timestamp_ms: u64,
            ) -> Self::Event {
                $event_ty {
                    id,
                    kind,
                    user_id,
                    task_id,
                    action_id,
                    payload,
                    correlation_id,
                    timestamp_ms,
                }
            }

            fn publish(&self, ev: &Self::Event) -> Result<(), String> {
                ($publish_fn)(ev).map_err(|e| e.to_string())
            }
        }
    };
}

/// Generate a standard WIT Guest entrypoint (`execute_action`) implemented on a
/// provided impl type.
///
/// This removes boilerplate around:
/// - parsing `action.params` JSON
/// - extracting `user_id/task_id/correlation_id` from `ActionContext`
/// - building `ActionRequest` + `ActionRuntime`
/// - mapping [`FrameworkOutcome`] back into WIT `ActionOutcome`
///
/// The component still controls:
/// - which [`ActionModule`] to dispatch to
/// - the concrete WIT types (so this is macro-based)
///
/// Usage (in a component crate):
/// ```ignore
/// ntx_action_sdk::define_wit_component_entry!(
///   impl_ty = ActionExecutorImpl,
///   guest_trait = exports::ntx::scenario_actions_executor::action_component::Guest,
///   action_def = ActionDef,
///   action_ctx = ActionContext,
///   action_outcome = ActionOutcome,
///   outcome_status = OutcomeStatus,
///   bus_ty = WITEventBus,
///   module_ty = ActionsExecutorModule
/// );
/// ```
#[macro_export]
macro_rules! define_wit_component_entry {
    // With optional `before_dispatch` hook.
    (
        impl_ty = $impl_ty:ty,
        guest_trait = $guest_trait:path,
        action_def = $action_def:ty,
        action_ctx = $action_ctx:ty,
        action_outcome = $action_outcome:ty,
        outcome_status = $outcome_status:ty,
        bus_ty = $bus_ty:ty,
        module_ty = $module_ty:ty,
        make_bus = $make_bus:expr,
        make_module = $make_module:expr,
        make_outcome = $make_outcome:expr,
        before_dispatch = $before_dispatch:expr
        $(,)?
    ) => {
        impl $guest_trait for $impl_ty {
            fn init_component() -> Result<(), String> {
                println!("[component] init-component");
                Ok(())
            }

            fn execute_action(
                action: $action_def,
                ctx: Option<$action_ctx>,
            ) -> Result<$action_outcome, String> {
                let user_id: Option<String> = ctx.as_ref().and_then(|c| c.user_id.clone());
                let task_id = ctx.as_ref().and_then(|c| c.task_id.clone());
                let correlation_id = ctx.as_ref().and_then(|c| c.correlation_id.clone());

                let params_json: serde_json::Value = serde_json::from_str(&action.params)
                    .map_err(|e| format!("parse params as json: {e}"))?;

                let req = $crate::ActionRequest {
                    id: action.id.clone(),
                    call: action.call.clone(),
                    params_json,
                };

                // Optional hook for logging / audit / tracing.
                // Signature suggestion:
                // |action: &$action_def, ctx: &Option<$action_ctx>, req: &$crate::ActionRequest| { ... }
                ($before_dispatch)(&action, &ctx, &req);

                let bus: $bus_ty = ($make_bus)();
                let rt = $crate::ActionRuntime {
                    bus: &bus,
                    ctx: $crate::ActionCtx {
                        user_id,
                        task_id,
                        correlation_id,
                    },
                };

                let module: $module_ty = ($make_module)();
                let out = module.handle(&rt, &req)?;

                let status: $outcome_status = match out.status {
                    $crate::FrameworkStatus::Success => <$outcome_status>::Success,
                    $crate::FrameworkStatus::Failed => <$outcome_status>::Failed,
                };

                Ok(($make_outcome)(status, out.detail, out.exports_json))
            }

            fn release_component() -> Result<(), String> {
                println!("[component] release-component");
                Ok(())
            }
        }
    };

    // Backwards-compatible form: no hook.
    (
        impl_ty = $impl_ty:ty,
        guest_trait = $guest_trait:path,
        action_def = $action_def:ty,
        action_ctx = $action_ctx:ty,
        action_outcome = $action_outcome:ty,
        outcome_status = $outcome_status:ty,
        bus_ty = $bus_ty:ty,
        module_ty = $module_ty:ty,
        make_bus = $make_bus:expr,
        make_module = $make_module:expr,
        make_outcome = $make_outcome:expr
        $(,)?
    ) => {
        $crate::define_wit_component_entry!(
            impl_ty = $impl_ty,
            guest_trait = $guest_trait,
            action_def = $action_def,
            action_ctx = $action_ctx,
            action_outcome = $action_outcome,
            outcome_status = $outcome_status,
            bus_ty = $bus_ty,
            module_ty = $module_ty,
            make_bus = $make_bus,
            make_module = $make_module,
            make_outcome = $make_outcome,
            before_dispatch = |_action: &$action_def,
                               _ctx: &Option<$action_ctx>,
                               _req: &$crate::ActionRequest| {},
        );
    };
}

/// Like [`define_wit_component_entry!`], but also calls an `after_dispatch` hook
/// with `(action, ctx, req, out)`.
///
/// This is intentionally macro-based because WIT types are component-local.
#[macro_export]
macro_rules! define_wit_component_entry_with_after_dispatch {
    (
        impl_ty = $impl_ty:ty,
        guest_trait = $guest_trait:path,
        action_def = $action_def:ty,
        action_ctx = $action_ctx:ty,
        action_outcome = $action_outcome:path,
        outcome_status = $outcome_status:ty,
        bus_ty = $bus_ty:ty,
        module_ty = $module_ty:ty,
        make_bus = $make_bus:expr,
        make_module = $make_module:expr,
        make_outcome = $make_outcome:expr,
        before_dispatch = $before_dispatch:expr,
        after_dispatch_tuple = $after_dispatch:expr
        $(,)?
    ) => {
        impl $guest_trait for $impl_ty {
            fn init_component() -> Result<(), String> {
                println!("[component] init-component");
                Ok(())
            }

            fn execute_action(
                action: $action_def,
                ctx: Option<$action_ctx>,
            ) -> Result<$action_outcome, String> {
                let user_id: Option<String> = ctx.as_ref().and_then(|c| c.user_id.clone());
                let task_id = ctx.as_ref().and_then(|c| c.task_id.clone());
                let correlation_id = ctx.as_ref().and_then(|c| c.correlation_id.clone());

                let params_json: serde_json::Value = serde_json::from_str(&action.params)
                    .map_err(|e| format!("parse params as json: {e}"))?;

                let req = $crate::ActionRequest {
                    id: action.id.clone(),
                    call: action.call.clone(),
                    params_json,
                };

                ($before_dispatch)(&action, &ctx, &req);

                let bus: $bus_ty = ($make_bus)();
                let rt = $crate::ActionRuntime {
                    bus: &bus,
                    ctx: $crate::ActionCtx {
                        user_id,
                        task_id,
                        correlation_id,
                    },
                };

                let module: $module_ty = ($make_module)();
                let out = module.handle(&rt, &req)?;

                // Force hook to a concrete fn pointer type to avoid callsite annotations.
                let hook: fn(
                    (
                        &$action_def,
                        &Option<$action_ctx>,
                        &$crate::ActionRequest,
                        &$crate::FrameworkOutcome,
                    ),
                ) = $after_dispatch;
                hook((&action, &ctx, &req, &out));

                let status: $outcome_status = match out.status {
                    $crate::FrameworkStatus::Success => <$outcome_status>::Success,
                    $crate::FrameworkStatus::Failed => <$outcome_status>::Failed,
                };

                Ok(($make_outcome)(status, out.detail, out.exports_json))
            }

            fn release_component() -> Result<(), String> {
                println!("[component] release-component");
                Ok(())
            }
        }
    };
}

/// Define a WIT component entrypoint with *minimal* parameters.
///
/// This macro is intentionally **not** backwards-compatible with the verbose
/// variants: the goal is to keep the public macro surface small and avoid code
/// bloat.
///
/// You provide:
/// - `types_mod`: module containing the WIT types (`ActionDef/ActionContext/ActionOutcome/OutcomeStatus`)
/// - `guest_trait`: the generated WIT `Guest` trait path
/// - `bus_ty` / `module_ty`: framework bus + module types
/// - optional `before_dispatch` hook
///
/// Defaults:
/// - `make_bus = || <$bus_ty>::default()`
/// - `make_module = || <$module_ty>::default()`
/// - `make_outcome = |status, detail, exports| $types_mod::ActionOutcome { status, detail, metrics: None, exports }`
///
/// Requirements:
/// - `$bus_ty: Default`
/// - `$module_ty: Default`
/// - `$types_mod::ActionOutcome` has fields `status`, `detail`, `metrics`, `exports`.
#[macro_export]
macro_rules! define_wit_component_entry_minimal {
    (
        impl_ty = $impl_ty:ty,
        guest_trait = $guest_trait:path,
        types_mod = ($($types_mod:tt)+),
        bus_ty = $bus_ty:ty,
        module_ty = $module_ty:ty,
        before_dispatch = $before_dispatch:expr
        $(,)?
    ) => {
        $crate::define_wit_component_entry!(
            impl_ty = $impl_ty,
            guest_trait = $guest_trait,
            action_def = $($types_mod)+::ActionDef,
            action_ctx = $($types_mod)+::ActionContext,
            action_outcome = $($types_mod)+::ActionOutcome,
            outcome_status = $($types_mod)+::OutcomeStatus,
            bus_ty = $bus_ty,
            module_ty = $module_ty,
            make_bus = || <$bus_ty>::default(),
            make_module = || <$module_ty>::default(),
            make_outcome = |status, detail, exports| {
                $($types_mod)+::ActionOutcome {
                    status,
                    detail,
                    metrics: None,
                    exports,
                }
            },
            // Allow the callsite to pass an untyped closure by wrapping it in a
            // typed shim that matches the underlying entry macro.
            before_dispatch = |action: &$($types_mod)+::ActionDef,
                               ctx: &Option<$($types_mod)+::ActionContext>,
                               req: &$crate::ActionRequest| {
                ($before_dispatch)(action, ctx, req)
            },
        );
    };

    // With after-dispatch hook (tuple function item), for unified logging/metrics.
    (
        impl_ty = $impl_ty:ty,
        guest_trait = $guest_trait:path,
        types_mod = ($($types_mod:tt)+),
        bus_ty = $bus_ty:ty,
        module_ty = $module_ty:ty,
        after_dispatch_tuple = $after_dispatch:expr
        $(,)?
    ) => {
        $crate::define_wit_component_entry_with_after_dispatch!(
            impl_ty = $impl_ty,
            guest_trait = $guest_trait,
            action_def = $($types_mod)+::ActionDef,
            action_ctx = $($types_mod)+::ActionContext,
            action_outcome = $($types_mod)+::ActionOutcome,
            outcome_status = $($types_mod)+::OutcomeStatus,
            bus_ty = $bus_ty,
            module_ty = $module_ty,
            make_bus = || <$bus_ty>::default(),
            make_module = || <$module_ty>::default(),
            make_outcome = |status, detail, exports| {
                $($types_mod)+::ActionOutcome {
                    status,
                    detail,
                    metrics: None,
                    exports,
                }
            },
            before_dispatch = |_action: &$($types_mod)+::ActionDef,
                               _ctx: &Option<$($types_mod)+::ActionContext>,
                               _req: &$crate::ActionRequest| {},
            after_dispatch_tuple = $after_dispatch,
        );
    };

    // With both hooks: before-dispatch tuple hook and after-dispatch tuple hook.
    (
        impl_ty = $impl_ty:ty,
        guest_trait = $guest_trait:path,
        types_mod = ($($types_mod:tt)+),
        bus_ty = $bus_ty:ty,
        module_ty = $module_ty:ty,
        before_dispatch_tuple = $before_dispatch:expr,
        after_dispatch_tuple = $after_dispatch:expr
        $(,)?
    ) => {
        $crate::define_wit_component_entry_with_after_dispatch!(
            impl_ty = $impl_ty,
            guest_trait = $guest_trait,
            action_def = $($types_mod)+::ActionDef,
            action_ctx = $($types_mod)+::ActionContext,
            action_outcome = $($types_mod)+::ActionOutcome,
            outcome_status = $($types_mod)+::OutcomeStatus,
            bus_ty = $bus_ty,
            module_ty = $module_ty,
            make_bus = || <$bus_ty>::default(),
            make_module = || <$module_ty>::default(),
            make_outcome = |status, detail, exports| {
                $($types_mod)+::ActionOutcome {
                    status,
                    detail,
                    metrics: None,
                    exports,
                }
            },
            before_dispatch = |action: &$($types_mod)+::ActionDef,
                               ctx: &Option<$($types_mod)+::ActionContext>,
                               req: &$crate::ActionRequest| {
                let hook: fn((
                    &$($types_mod)+::ActionDef,
                    &Option<$($types_mod)+::ActionContext>,
                    &$crate::ActionRequest,
                )) = $before_dispatch;
                hook((action, ctx, req))
            },
            after_dispatch_tuple = $after_dispatch,
        );
    };

    // Convenience: allow a single-arg closure that takes a typed tuple.
    // This avoids needing multi-arg type inference at the callsite.
    (
        impl_ty = $impl_ty:ty,
        guest_trait = $guest_trait:path,
        types_mod = ($($types_mod:tt)+),
        bus_ty = $bus_ty:ty,
        module_ty = $module_ty:ty,
        before_dispatch_tuple = $before_dispatch:expr
        $(,)?
    ) => {
        $crate::define_wit_component_entry!(
            impl_ty = $impl_ty,
            guest_trait = $guest_trait,
            action_def = $($types_mod)+::ActionDef,
            action_ctx = $($types_mod)+::ActionContext,
            action_outcome = $($types_mod)+::ActionOutcome,
            outcome_status = $($types_mod)+::OutcomeStatus,
            bus_ty = $bus_ty,
            module_ty = $module_ty,
            make_bus = || <$bus_ty>::default(),
            make_module = || <$module_ty>::default(),
            make_outcome = |status, detail, exports| {
                $($types_mod)+::ActionOutcome {
                    status,
                    detail,
                    metrics: None,
                    exports,
                }
            },
            before_dispatch = |action: &$($types_mod)+::ActionDef,
                               ctx: &Option<$($types_mod)+::ActionContext>,
                               req: &$crate::ActionRequest| {
                // Force the callsite hook to be a concrete function pointer type,
                // which avoids needing type annotations at the callsite.
                let hook: fn((
                    &$($types_mod)+::ActionDef,
                    &Option<$($types_mod)+::ActionContext>,
                    &$crate::ActionRequest,
                )) = $before_dispatch;

                hook((action, ctx, req))
            },
        );
    };

    (
        impl_ty = $impl_ty:ty,
        guest_trait = $guest_trait:path,
        types_mod = ($($types_mod:tt)+),
        bus_ty = $bus_ty:ty,
        module_ty = $module_ty:ty
        $(,)?
    ) => {
        $crate::define_wit_component_entry_minimal!(
            impl_ty = $impl_ty,
            guest_trait = $guest_trait,
            types_mod = ($($types_mod)+),
            bus_ty = $bus_ty,
            module_ty = $module_ty,
            before_dispatch = |_action: &$($types_mod)+::ActionDef,
                               _ctx: &Option<$($types_mod)+::ActionContext>,
                               _req: &$crate::ActionRequest| {},
        );
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_payload_hex() {
        let v = serde_json::json!({"payload_hex":"0a0b"});
        assert_eq!(
            parse_payload_spec(&v).unwrap(),
            PayloadSpec::Hex("0a0b".to_string())
        );
    }

    #[test]
    fn parse_payload_bytes() {
        let v = serde_json::json!({"payload_bytes":[1,2,255]});
        assert_eq!(
            parse_payload_spec(&v).unwrap(),
            PayloadSpec::Bytes(vec![1, 2, 255])
        );
    }

    #[test]
    fn hex_decoder_allows_0x_and_ws() {
        assert_eq!(
            decode_hex_bytes("0x0a 0b\n0C").unwrap(),
            vec![0x0a, 0x0b, 0x0c]
        );
    }

    #[test]
    fn schedule_periodic() {
        let v = serde_json::json!({"mode":"periodic","interval_ms":100,"start_delay_ms":50});
        assert_eq!(
            parse_schedule_like(&v).unwrap(),
            ScheduleLike::Periodic {
                interval_ms: 100,
                start_delay_ms: Some(50)
            }
        );
    }

    #[test]
    fn params_alias_resolution() {
        let v = serde_json::json!({
            "max-count": 7,
            "timeout_ms": 12,
            "request-id": "r1",
        });

        assert_eq!(params::opt_u32(&v, &["max_count", "max-count"]), Some(7));
        assert_eq!(params::opt_u64(&v, &["timeout_ms", "timeout-ms"]), Some(12));
        assert_eq!(
            params::opt_string(&v, &["request_id", "request-id"]),
            Some("r1".to_string())
        );
    }

    #[test]
    fn params_required_missing_has_good_error() {
        let v = serde_json::json!({});
        let e = params::req_u64(&v, &["socket_id"]).unwrap_err();
        assert!(e.contains("missing"));
        assert!(e.contains("socket_id"));
    }

    #[test]
    fn params_required_type_mismatch() {
        let v = serde_json::json!({"socket_id": "nope"});
        let e = params::req_u64(&v, &["socket_id"]).unwrap_err();
        assert!(e.contains("must be"));
    }

    #[test]
    fn normalize_param_keys_adds_kebab_alias() {
        let v = serde_json::json!({
            "max-count": 1,
            "nested": {"timeout-ms": 2},
            "arr": [{"request-id": "r1"}],
        });

        let n = normalize_param_keys(&v);
        assert_eq!(n.get("max_count").and_then(|x| x.as_u64()), Some(1));
        assert_eq!(
            n.get("nested")
                .and_then(|x| x.get("timeout_ms"))
                .and_then(|x| x.as_u64()),
            Some(2)
        );
        assert_eq!(
            n.get("arr")
                .and_then(|x| x.get(0))
                .and_then(|x| x.get("request_id"))
                .and_then(|x| x.as_str()),
            Some("r1")
        );
    }

    #[test]
    fn parse_params_typed_supports_kebab_aliases() {
        #[derive(Debug, serde::Deserialize)]
        struct P {
            socket_id: u64,
            max_count: Option<u32>,
        }

        let req = ActionRequest {
            id: "a1".to_string(),
            call: "udp.schedule-send".to_string(),
            params_json: serde_json::json!({"socket_id": 9, "max-count": 10}),
        };

        let p: P = parse_params(&req).unwrap();
        assert_eq!(p.socket_id, 9);
        assert_eq!(p.max_count, Some(10));
    }
}
