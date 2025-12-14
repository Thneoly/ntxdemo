//! Helpers for calling component-model guest packet handlers using dynamic `Val`.
//!
//! This is intentionally small and only supports the shapes we use in MVP-0:
//! `result<option<udp-response>, string>` where `udp-response` is `{ payload: list<u8> }`.

use anyhow::{Result, anyhow};
use wasmtime::component::Val;

/// Decode `result<option<udp-response>, string>` as returned by packet/on-udp.
///
/// Returns:
/// - Ok(Some(payload)) => guest wants us to reply with payload
/// - Ok(None) => guest wants us to drop/ignore
/// - Err(msg) => guest returned an error (or unexpected shape)
pub fn parse_on_udp_result(results: &[Val]) -> std::result::Result<Option<Vec<u8>>, String> {
    if results.len() != 1 {
        return Err(format!("expected 1 result, got {}", results.len()));
    }

    // Wasmtime has used multiple encodings for component `result<T, E>` over time.
    // Support both:
    // - Old: Val::Variant("ok"|"err", payload)
    // - New: Val::Result(Ok(Some(v))|Ok(None)|Err(Some(v))|Err(None))
    match &results[0] {
        Val::Variant(name, payload) => match name.as_str() {
            "ok" => {
                let v = payload
                    .as_deref()
                    .ok_or_else(|| "result.ok missing payload".to_string())?;
                parse_ok_option_response(v)
            }
            "err" => {
                let v = payload
                    .as_deref()
                    .ok_or_else(|| "result.err missing payload".to_string())?;
                Err(format_variant_payload(v))
            }
            other => Err(format!("unexpected result variant tag: {other}")),
        },
        Val::Result(r) => match r {
            Ok(Some(v)) => parse_ok_option_response(v),
            Ok(None) => Err("result.ok missing payload".to_string()),
            Err(Some(v)) => Err(format_variant_payload(v)),
            Err(None) => Err("result.err missing payload".to_string()),
        },
        other => Err(format!("unexpected result encoding: {other:?}")),
    }
}

fn parse_ok_option_response(ok_payload: &Val) -> std::result::Result<Option<Vec<u8>>, String> {
    // Wasmtime used multiple encodings for option<T>:
    // - Old: Val::Variant("none"|"some", payload)
    // - New: Val::Option(Option<Box<Val>>)
    let inner: Option<&Val> = match ok_payload {
        Val::Variant(name, payload) => match name.as_str() {
            "none" => return Ok(None),
            "some" => Some(
                payload
                    .as_deref()
                    .ok_or_else(|| "option.some missing payload".to_string())?,
            ),
            other => return Err(format!("unexpected option variant tag: {other}")),
        },
        Val::Option(v) => v.as_deref(),
        other => {
            return Err(format!(
                "option encoding expected Variant/Option, got {other:?}"
            ));
        }
    };

    let Some(v) = inner else {
        return Ok(None);
    };

    // udp-response is record { payload: list<u8> }
    match v {
        Val::Record(fields) => {
            let payload_field = fields
                .iter()
                .find(|(k, _)| k == "payload")
                .map(|(_, v)| v)
                .ok_or_else(|| "udp-response missing 'payload' field".to_string())?;

            match payload_field {
                Val::List(items) => {
                    let mut out = Vec::with_capacity(items.len());
                    for v in items {
                        match v {
                            Val::U8(b) => out.push(*b),
                            other => {
                                return Err(format!("payload element expected u8, got {other:?}"));
                            }
                        }
                    }
                    Ok(Some(out))
                }
                other => Err(format!("payload field expected list<u8>, got {other:?}")),
            }
        }
        other => Err(format!("udp-response expected record, got {other:?}")),
    }
}

fn format_variant_payload(v: &Val) -> String {
    // Best-effort stringify. If it's a string, it will be Val::String.
    match v {
        Val::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

#[allow(dead_code)]
pub fn ensure_shape(_results: &[Val]) -> Result<()> {
    // Placeholder for future shape checks.
    Err(anyhow!("not implemented"))
}
