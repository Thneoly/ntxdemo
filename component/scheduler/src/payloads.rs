//! Typed payloads for events published/consumed by the scheduler.
//!
//! Goal: remove ad-hoc `serde_json::json!({ ... })` blobs for payloads that
//! have a stable schema, while keeping the emitted JSON backward-compatible.

use crate::codec;

/// Stable payload schema for `packet.rx` events.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PacketRxEventPayload {
    pub(crate) sock_id: u64,
    pub(crate) seq: u64,
    pub(crate) len: usize,
    pub(crate) payload_hex: String,
    pub(crate) ts_ms: u64,
}

impl PacketRxEventPayload {
    pub(crate) fn from_bytes(sock_id: u64, seq: u64, payload: &[u8], ts_ms: u64) -> Self {
        Self {
            sock_id,
            seq,
            len: payload.len(),
            payload_hex: codec::to_hex(payload),
            ts_ms,
        }
    }
}

/// Fixed "reason" values that appear in evaluation contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalReason {
    Success,
    Timeout,
    Failed,
    PacketRx,
}

impl EvalReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            EvalReason::Success => "success",
            EvalReason::Timeout => "timeout",
            EvalReason::Failed => "failed",
            EvalReason::PacketRx => "packet.rx",
        }
    }
}

/// Stable eval context schema used by workflow trigger evaluation.
///
/// NOTE: We intentionally keep this as a struct (not a tagged enum) because
/// downstream condition evaluation expects simple JSON paths.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct EvalCtx {
    pub(crate) event: String,
    pub(crate) reason: String,

    pub(crate) user_id: String,
    pub(crate) task_id: String,
    pub(crate) action_id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sock_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) len: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) payload_hex: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) exports: Option<serde_json::Value>,
}

impl EvalCtx {
    pub(crate) fn packet_rx(
        user_id: &str,
        task_id: &str,
        action_id: &str,
        sock_id: u64,
        len: u64,
        payload_hex: &str,
    ) -> Self {
        Self {
            event: crate::EventKind::PacketRx.as_str().to_string(),
            reason: EvalReason::PacketRx.as_str().to_string(),
            user_id: user_id.to_string(),
            task_id: task_id.to_string(),
            action_id: action_id.to_string(),
            sock_id: Some(sock_id),
            len: Some(len),
            payload_hex: Some(payload_hex.to_string()),
            status: None,
            detail: None,
            exports: None,
        }
    }

    pub(crate) fn action_result(
        user_id: &str,
        task_id: &str,
        action_id: &str,
        reason: EvalReason,
        status: &str,
        detail: serde_json::Value,
        exports: serde_json::Value,
    ) -> Self {
        Self {
            event: crate::EventKind::SchedulerActionResult.as_str().to_string(),
            reason: reason.as_str().to_string(),
            user_id: user_id.to_string(),
            task_id: task_id.to_string(),
            action_id: action_id.to_string(),
            sock_id: None,
            len: None,
            payload_hex: None,
            status: Some(status.to_string()),
            detail: Some(detail),
            exports: Some(exports),
        }
    }
}
