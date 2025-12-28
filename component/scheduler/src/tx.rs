//! packet.tx-request handling and UDP transmit helpers.

use crate::ntx::host::udp_socket_control;

/// Parse a tx-request JSON payload and transmit via `udp_socket_control`.
pub fn handle_tx_request(payload_json: &str, correlation_id: Option<&str>) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct TxReq {
        sock_id: u64,
        #[serde(default)]
        payload: Option<String>,
        #[serde(default)]
        payload_hex: Option<String>,
        #[serde(default)]
        payload_bytes: Option<Vec<u8>>,
        #[serde(default)]
        user_id: Option<String>,
        #[serde(default)]
        task_id: Option<String>,
        #[serde(default)]
        action_id: Option<String>,
    }

    fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
        let mut t = s.trim().to_ascii_lowercase();
        if let Some(rest) = t.strip_prefix("0x") {
            t = rest.to_string();
        }
        let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        if t.len() % 2 != 0 {
            return Err("payload_hex length must be even".to_string());
        }
        let bytes = t
            .as_bytes()
            .chunks(2)
            .map(|pair| {
                let hi = pair[0] as char;
                let lo = pair[1] as char;
                let hex = [hi, lo].iter().collect::<String>();
                u8::from_str_radix(&hex, 16).map_err(|_| format!("invalid hex byte: {hex}"))
            })
            .collect::<Result<Vec<u8>, String>>()?;
        Ok(bytes)
    }

    let req: TxReq = serde_json::from_str(payload_json)
        .map_err(|e| format!("parse tx-request payload json: {e}"))?;

    let payload: Vec<u8> = if let Some(b) = req.payload_bytes.clone() {
        b
    } else if let Some(h) = req.payload_hex.as_deref() {
        decode_hex(h)?
    } else if let Some(s) = req.payload.clone() {
        s.into_bytes()
    } else {
        return Err("missing payload: expected payload / payload_hex / payload_bytes".to_string());
    };

    send_udp(
        req.sock_id,
        &payload,
        req.user_id.as_deref(),
        req.task_id.as_deref(),
        req.action_id.as_deref(),
        correlation_id,
    )
}

/// In socket close: remove ctx for this sock_id.
pub fn clear_sock_ctx_for_socket(sock_id: u64) {
    if let Ok(mut map) = crate::SOCK_CTX.lock() {
        if let Some(prev) = map.remove(&sock_id) {
            println!(
                "[scheduler][sock_ctx] remove(sock): sock_id={} user_id={:?} task_id={:?} action_id={:?} corr_id={:?}",
                sock_id,
                prev.user_id,
                prev.task_id,
                prev.action_id,
                prev.correlation_id
            );
        } else {
            println!(
                "[scheduler][sock_ctx] remove(sock): sock_id={} (not found)",
                sock_id
            );
        }
    }
}

/// In user finish: remove any ctx belonging to that user.
pub fn clear_sock_ctx_for_user(user_id: &str) {
    if let Ok(mut map) = crate::SOCK_CTX.lock() {
        let before = map.len();
        map.retain(|sock_id, ctx| {
            let is_match = ctx.user_id.as_deref() == Some(user_id);
            if is_match {
                println!(
                    "[scheduler][sock_ctx] remove(user): sock_id={} user_id={:?} task_id={:?} action_id={:?} corr_id={:?}",
                    sock_id,
                    ctx.user_id,
                    ctx.task_id,
                    ctx.action_id,
                    ctx.correlation_id
                );
            }
            !is_match
        });
        let removed = before.saturating_sub(map.len());
        if removed > 0 {
            println!(
                "[scheduler][sock_ctx] remove(user): user_id={} removed={} remaining={}",
                user_id,
                removed,
                map.len()
            );
        }
    }
}

pub fn send_udp(
    sock_id: u64,
    payload: &[u8],
    user_id: Option<&str>,
    task_id: Option<&str>,
    action_id: Option<&str>,
    correlation_id: Option<&str>,
) -> Result<(), String> {
    let now_ms = crate::time::now_ms();

    // record sock context for later packet.rx correlation
    if let Ok(mut map) = crate::SOCK_CTX.lock() {
        let new_ctx = crate::SockCtx {
            user_id: user_id.map(|s| s.to_string()),
            task_id: task_id.map(|s| s.to_string()),
            action_id: action_id.map(|s| s.to_string()),
            correlation_id: correlation_id.map(|s| s.to_string()),
            last_seen_ms: now_ms,
        };

        match map.insert(sock_id, new_ctx.clone()) {
            None => {
                println!(
                    "[scheduler][sock_ctx] add: sock_id={} user_id={:?} task_id={:?} action_id={:?} corr_id={:?}",
                    sock_id,
                    new_ctx.user_id,
                    new_ctx.task_id,
                    new_ctx.action_id,
                    new_ctx.correlation_id
                );
            }
            Some(prev) => {
                // Overwrite is expected when the same socket is reused across actions.
                println!(
                    "[scheduler][sock_ctx] update: sock_id={} prev(user_id={:?} task_id={:?} action_id={:?} corr_id={:?}) -> new(user_id={:?} task_id={:?} action_id={:?} corr_id={:?})",
                    sock_id,
                    prev.user_id,
                    prev.task_id,
                    prev.action_id,
                    prev.correlation_id,
                    new_ctx.user_id,
                    new_ctx.task_id,
                    new_ctx.action_id,
                    new_ctx.correlation_id
                );
            }
        }
    }

    let frame = udp_socket_control::build_reply(sock_id, payload)
        .map_err(|e| format!("build_reply failed: {:?}", e))?;
    udp_socket_control::tx(frame).map_err(|e| format!("tx failed: {:?}", e))?;

    Ok(())
}
