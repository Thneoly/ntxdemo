//! RX ring decode and `packet.rx` event publishing.

use std::sync::atomic::Ordering;

use crate::codec;

// --- rx decode constants ---
// This module is currently not wired into the main event loop in all builds.
// Keep it available for host/guest integration, but don't spam warnings.
#[allow(dead_code)]
pub(crate) const NTX_MAGIC: u32 = 0x4E54_5830; // "NTX0"
#[allow(dead_code)]
pub(crate) const NTX_VERSION: u16 = 1;
#[allow(dead_code)]
pub(crate) const CONTROL_LEN: usize = 48;
#[allow(dead_code)]
pub(crate) const DESC_LEN: usize = 32;
#[allow(dead_code)]
pub(crate) const DESCS_OFF: usize = 0x1000;
#[allow(dead_code)]
pub(crate) const MAX_CONSUME: u32 = 64;

#[derive(Debug, Clone, Copy)]
struct ControlBlockV1 {
    desc_capacity: u32,
    desc_head: u32,
    desc_tail: u32,
    payload_capacity: u32,
    payload_head: u32,
    payload_tail: u32,
}

fn decode_control_v1(desc_mem: &[u8]) -> Result<ControlBlockV1, String> {
    if desc_mem.len() < CONTROL_LEN {
        return Err(format!(
            "desc_mem too small for control block: len={} need={}",
            desc_mem.len(),
            CONTROL_LEN
        ));
    }

    let magic = codec::le_u32(&desc_mem[0..4]);
    let version = codec::le_u16(&desc_mem[4..6]);
    if magic != NTX_MAGIC || version != NTX_VERSION {
        return Err(format!(
            "invalid magic/version: {:08X}/{:04X}",
            magic, version
        ));
    }

    // Host layout (src/rx_layout.rs):
    //  8..12  desc_capacity
    // 12..16  desc_head
    // 16..20  desc_tail
    // 20..24  payload_capacity
    // 24..28  payload_head
    // 28..32  payload_tail
    let desc_capacity = codec::le_u32(&desc_mem[8..12]);
    let desc_head = codec::le_u32(&desc_mem[12..16]);
    let desc_tail = codec::le_u32(&desc_mem[16..20]);
    let payload_capacity = codec::le_u32(&desc_mem[20..24]);
    let payload_head = codec::le_u32(&desc_mem[24..28]);
    let payload_tail = codec::le_u32(&desc_mem[28..32]);

    Ok(ControlBlockV1 {
        desc_capacity,
        desc_head,
        desc_tail,
        payload_capacity,
        payload_head,
        payload_tail,
    })
}

#[allow(dead_code)]
pub(crate) static PACKET_RX_SEQ: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// Ingest descriptors + payload shared memory and publish `packet.rx` events.
///
/// Returns number of descriptors consumed.
#[allow(dead_code)]
pub fn drain_rx_ring(desc_mem: Vec<u8>, payload_mem: Vec<u8>) -> u32 {
    println!("[scheduler] drain_rx_ring: called");
    let cb = match decode_control_v1(&desc_mem) {
        Ok(cb) => cb,
        Err(e) => {
            println!("[scheduler] drain_rx_ring: control decode failed: {e}");
            return 0;
        }
    };

    let desc_capacity = cb.desc_capacity as usize;
    let mut head = cb.desc_head as usize;
    let tail = cb.desc_tail as usize;

    if desc_capacity == 0 {
        println!("[scheduler] drain_rx_ring: desc_capacity is 0");
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

        // Host layout uses PAYLOAD_OFF=0, so payload_off is relative to payload_mem[0].
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
                        // Refresh last_seen and emit a low-volume trace for correlation.
                        // This is intentionally `println!` (not `tracing`) because the guest
                        // component often runs without a tracing subscriber.
                        //
                        // Note: this will be noisy under high packet rates; if needed we can
                        // add sampling or a per-sock rate limit later.
                        println!(
                            "[scheduler][sock_ctx] touch(rx): sock_id={:02X} user_id={:?} task_id={:?} action_id={:?} corr_id={:?}",
                            sock_id,
                            c.user_id,
                            c.task_id,
                            c.action_id,
                            c.correlation_id
                        );
                        c.last_seen_ms = now_ms;
                        c.clone()
                    })
            };

            if ctx.is_none() {
                println!(
                    "[scheduler][sock_ctx] miss(rx): sock_id={:02X} (no mapping)",
                    sock_id
                );
            }

            publish_packet_event(sock_id, payload, ctx.as_ref(), now_ms);
        }

        // Debug signal for layout mismatch: descriptor points outside payload buffer.
        // (Most commonly caused by DESCS_OFF/CONTROL_LEN mismatch between host & guest.)
        if payload_off + payload_len > payload_mem.len() {
            println!(
                "[scheduler] drain_rx_ring: desc payload out of bounds: sock_id={} payload_off={} payload_len={} payload_mem_len={} cb_payload_tail={}",
                sock_id,
                payload_off,
                payload_len,
                payload_mem.len(),
                cb.payload_tail
            );
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
    println!(
        "[scheduler] publish_packet_event: sock_id={}, seq={}, len={}",
        sock_id,
        seq,
        payload.len()
    );
    let res = crate::ntx::scenario_eventbus::event_bus::publish(
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
    if let Err(e) = res {
        println!(
            "[scheduler] publish_packet_event: failed to publish event: {}",
            e
        );
    }
}
