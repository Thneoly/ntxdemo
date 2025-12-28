//! RX ring decode and `packet.rx` event publishing.

use std::sync::atomic::Ordering;

use crate::codec;

// --- rx decode constants ---
// This module is currently not wired into the main event loop in all builds.
// Keep it available for host/guest integration, but don't spam warnings.
#[allow(dead_code)]
pub(crate) const NTX_MAGIC: u32 = 0x4E_54_58_00; // "NTX\0"
#[allow(dead_code)]
pub(crate) const NTX_VERSION: u16 = 1;
#[allow(dead_code)]
pub(crate) const CONTROL_LEN: usize = 16;
#[allow(dead_code)]
pub(crate) const DESC_LEN: usize = 32;
#[allow(dead_code)]
pub(crate) const DESCS_OFF: usize = CONTROL_LEN;
#[allow(dead_code)]
pub(crate) const MAX_CONSUME: u32 = 64;

#[allow(dead_code)]
pub(crate) static PACKET_RX_SEQ: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// Ingest descriptors + payload shared memory and publish `packet.rx` events.
///
/// Returns number of descriptors consumed.
#[allow(dead_code)]
pub fn drain_rx_ring(desc_mem: Vec<u8>, payload_mem: Vec<u8>) -> u32 {
    if desc_mem.len() < CONTROL_LEN {
        return 0;
    }

    let magic = codec::le_u32(&desc_mem[0..4]);
    let version = codec::le_u16(&desc_mem[4..6]);
    if magic != NTX_MAGIC || version != NTX_VERSION {
        return 0;
    }

    let desc_capacity = codec::le_u32(&desc_mem[8..12]) as usize;
    let mut head = codec::le_u32(&desc_mem[12..16]) as usize;
    let tail = codec::le_u32(&desc_mem[16..20]) as usize;

    if desc_capacity == 0 {
        return 0;
    }

    let mut consumed: u32 = 0;

    while head != tail {
        let idx = head % desc_capacity;
        let base = DESCS_OFF + idx * DESC_LEN;
        if base + DESC_LEN > desc_mem.len() {
            break;
        }

        let desc = &desc_mem[base..base + DESC_LEN];
        let sock_id = codec::le_u64(&desc[0..8]);
        let payload_off = codec::le_u32(&desc[8..12]) as usize;
        let payload_len = codec::le_u32(&desc[12..16]) as usize;

        if payload_off + payload_len <= payload_mem.len() {
            let payload = &payload_mem[payload_off..payload_off + payload_len];

            // Lookup user/task/action correlation by sock_id; refresh last_seen_ms.
            let now_ms = crate::time::now_ms();
            let ctx = {
                let mut guard = crate::SOCK_CTX.lock().ok();
                guard
                    .as_mut()
                    .and_then(|map| map.get_mut(&sock_id))
                    .map(|c| {
                        c.last_seen_ms = now_ms;
                        c.clone()
                    })
            };

            publish_packet_event(sock_id, payload, ctx.as_ref(), now_ms);
        }

        head = head.wrapping_add(1);
        consumed += 1;
        if consumed >= MAX_CONSUME {
            break;
        }
    }

    consumed
}

#[allow(dead_code)]
fn publish_packet_event(sock_id: u64, payload: &[u8], ctx: Option<&crate::SockCtx>, now_ms: u64) {
    let id = format!(
        "rx-{}",
        crate::EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let seq = PACKET_RX_SEQ.fetch_add(1, Ordering::Relaxed);

    let json_payload = serde_json::json!({
        "sock_id": sock_id,
        "seq": seq,
        "len": payload.len(),
        "payload_hex": codec::to_hex(payload),
        "ts_ms": now_ms,
    })
    .to_string();

    let _ = crate::ntx::scenario_eventbus::event_bus::publish(
        &crate::ntx::scenario_eventbus::event_bus::Event {
            id,
            kind: "packet.rx".to_string(),
            user_id: ctx.and_then(|c| c.user_id.clone()),
            task_id: ctx.and_then(|c| c.task_id.clone()),
            action_id: ctx.and_then(|c| c.action_id.clone()),
            payload: json_payload,
            correlation_id: ctx.and_then(|c| c.correlation_id.clone()),
            timestamp_ms: now_ms,
        },
    );
}
