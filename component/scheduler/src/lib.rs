//! 调度器组件骨架（wasm32-wasip2）。
//! 仅提供占位实现，便于后续对接状态机与负载控制逻辑。

wit_bindgen::generate!({
    world: "scheduler:main/scheduler-main@0.2.0",
    path: [
        "../wit/core-types",
        "../wit/eventbus",
        "../wit/host",
        "../wit/protocol",
        "../wit/scheduler",
    ],
    generate_all,
    debug: true,
});

struct SchedulerExports;

impl exports::scheduler::main::scheduler_component::Guest for SchedulerExports {
    fn run(config_dir: String) -> Result<(), String> {
        // 占位：真实实现应在此加载 workflow/workbook/load 配置并进入事件循环。
        println!("[scheduler] run with config dir: {config_dir}");
        Ok(())
    }
}

impl exports::scheduler::main::send_scheduler::Guest for SchedulerExports {
    fn schedule_send(
        request: exports::scheduler::main::send_scheduler::SendRequest,
    ) -> Result<String, String> {
        // 直接回显 request-id，后续可接入计时器/速率控制。
        Ok(request.request_id.clone())
    }

    fn cancel_send(_request_id: String) -> Result<(), String> {
        Ok(())
    }

    fn query_send_status(
        request_id: String,
    ) -> Result<exports::scheduler::main::send_scheduler::SendStatus, String> {
        Ok(exports::scheduler::main::send_scheduler::SendStatus {
            request_id,
            state: exports::scheduler::main::send_scheduler::SendRequestState::Pending,
            total_sent: 0,
            last_sent_time: None,
            next_send_time: None,
        })
    }
}

impl exports::scheduler::main::packet_ingest::Guest for SchedulerExports {
    fn notify_rx(desc_mem: Vec<u8>, payload_mem: Vec<u8>) -> Result<u32, String> {
        Ok(drain_rx_ring(desc_mem, payload_mem))
    }
}

impl exports::scheduler::main::packet_tx::Guest for SchedulerExports {
    fn process_tx_request(payload_json: String) -> Result<(), String> {
        handle_tx_request(&payload_json)
    }
}

export!(SchedulerExports);

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const NTX_MAGIC: u32 = 0x4E54_5830; // "NTX0"
const NTX_VERSION: u16 = 1;
const CONTROL_LEN: usize = 48;
const DESC_LEN: usize = 32;
const DESCS_OFF: usize = 0x1000;
const MAX_CONSUME: u32 = 64;

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);
#[derive(Clone)]
struct SockCtx {
    user_id: Option<String>,
    task_id: Option<String>,
    action_id: Option<String>,
}

static SOCK_CTX: Lazy<Mutex<HashMap<u64, SockCtx>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn le_u16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

fn le_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

fn to_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for byte in data {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", byte);
    }
    s
}

/// 参考 packet-engine 的 drain_rx_ring：读取 desc ring，生成事件到 eventbus。
fn drain_rx_ring(desc_mem: Vec<u8>, payload_mem: Vec<u8>) -> u32 {
    if desc_mem.len() < CONTROL_LEN {
        return 0;
    }

    let magic = le_u32(&desc_mem[0..4]);
    let version = le_u16(&desc_mem[4..6]);
    if magic != NTX_MAGIC || version != NTX_VERSION {
        return 0;
    }

    let desc_capacity = le_u32(&desc_mem[8..12]) as usize;
    let mut head = le_u32(&desc_mem[12..16]) as usize;
    let tail = le_u32(&desc_mem[16..20]) as usize;

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
        let sock_id = le_u64(&desc[0..8]);
        let payload_off = le_u32(&desc[8..12]) as usize;
        let payload_len = le_u32(&desc[12..16]) as usize;

        if payload_off + payload_len <= payload_mem.len() {
            let payload = &payload_mem[payload_off..payload_off + payload_len];

            // 根据 sock_id 查找 user/task/action 关联
            let ctx = SOCK_CTX
                .lock()
                .ok()
                .and_then(|map| map.get(&sock_id).cloned());
            publish_packet_event(sock_id, payload, ctx.as_ref());
        }

        head = head.wrapping_add(1);
        consumed += 1;
        if consumed >= MAX_CONSUME {
            break;
        }
    }

    consumed
}

fn publish_packet_event(sock_id: u64, payload: &[u8], ctx: Option<&SockCtx>) {
    let id = format!("rx-{}", EVENT_COUNTER.fetch_add(1, Ordering::Relaxed));
    let payload_hex = to_hex(payload);
    let json_payload = format!(
        "{{\"sock_id\":{},\"payload_hex\":\"{}\",\"len\":{}}}",
        sock_id,
        payload_hex,
        payload.len()
    );

    let _ = scheduler::event_bus::event_bus::publish(&scheduler::event_bus::event_bus::Event {
        id,
        kind: "packet.rx".to_string(),
        user_id: ctx.and_then(|c| c.user_id.clone()),
        task_id: ctx.and_then(|c| c.task_id.clone()),
        action_id: ctx.and_then(|c| c.action_id.clone()),
        payload: json_payload,
        correlation_id: None,
        timestamp_ms: 0,
    });
}

fn handle_tx_request(payload_json: &str) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct TxReq {
        sock_id: u64,
        payload: String,
        user_id: Option<String>,
        task_id: Option<String>,
        action_id: Option<String>,
    }

    let req: TxReq = serde_json::from_str(payload_json)
        .map_err(|e| format!("parse tx-request payload json: {e}"))?;

    // 记录 sock 上下文用于后续 packet.rx 关联
    {
        if let Ok(mut map) = SOCK_CTX.lock() {
            map.insert(
                req.sock_id,
                SockCtx {
                    user_id: req.user_id.clone(),
                    task_id: req.task_id.clone(),
                    action_id: req.action_id.clone(),
                },
            );
        }
    }

    let frame = ntx::hostnet::udp_socket_control::build_reply(req.sock_id, req.payload.as_bytes())
        .map_err(|e| format!("build_reply failed: {:?}", e))?;
    ntx::hostnet::udp_socket_control::tx(frame)
        .map_err(|e| format!("tx failed: {:?}", e))?;
    Ok(())
}
