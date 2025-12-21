wit_bindgen::generate!({
    world: "ntx:packet/packet-engine",
    path: ["../wit/host","../wit/packet-engine"],
    generate_all,
    debug: true,
});

// Shared-memory ABI constants should match host's `ntx::wasm_engine::shared_mem`.
const NTX_MAGIC: u32 = 0x4E54_5830; // "NTX0"
const NTX_VERSION: u16 = 1;

const CONTROL_LEN: usize = 48;
const DESC_LEN: usize = 32;

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn le_u16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn le_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}
fn write_le_u32(dst: &mut [u8], v: u32) {
    dst.copy_from_slice(&v.to_le_bytes());
}

struct Component;

impl Guest for Component {
    fn desc_get() -> Vec<u8> {
        DESC.with(|b| b.borrow().clone())
    }

    fn desc_put(off: u32, data: Vec<u8>) {
        let off = off as usize;
        DESC.with(|b| {
            let mut buf = b.borrow_mut();
            if off >= buf.len() {
                return;
            }
            let n = data.len().min(buf.len() - off);
            buf[off..off + n].copy_from_slice(&data[..n]);
        })
    }

    fn payload_get() -> Vec<u8> {
        PAYLOAD.with(|b| b.borrow().clone())
    }

    fn payload_put(off: u32, data: Vec<u8>) {
        let off = off as usize;
        PAYLOAD.with(|b| {
            let mut buf = b.borrow_mut();
            if off >= buf.len() {
                return;
            }
            let n = data.len().min(buf.len() - off);
            buf[off..off + n].copy_from_slice(&data[..n]);
        })
    }

    fn notify_rx() -> u32 {
        let mut dmem = DESC.with(|b| b.borrow().clone());
        let pmem = PAYLOAD.with(|b| b.borrow().clone());

        if dmem.len() < CONTROL_LEN {
            return 0;
        }

        // Parse control block at 0.
        let magic = le_u32(&dmem[0..4]);
        let version = le_u16(&dmem[4..6]);
        if magic != NTX_MAGIC || version != NTX_VERSION {
            return 0;
        }

        let desc_capacity = le_u32(&dmem[8..12]) as usize;
        let mut head = le_u32(&dmem[12..16]) as usize;
        let tail = le_u32(&dmem[16..20]) as usize;

        // Ring starts at offset 0x1000 in desc memory (keep consistent with host v1).
        let descs_off: usize = 0x1000;

        if desc_capacity == 0 {
            return 0;
        }

        let mut consumed: u32 = 0;
        // Host enqueues by advancing `desc_tail`; guest consumes by advancing `desc_head`.
        while head != tail {
            let idx = head % desc_capacity;
            let base = descs_off + idx * DESC_LEN;
            if base + DESC_LEN > dmem.len() {
                break;
            }

            let desc = &dmem[base..base + DESC_LEN];
            let sock_id = le_u64(&desc[0..8]);
            let payload_off = le_u32(&desc[8..12]) as usize;
            let payload_len = le_u32(&desc[12..16]) as usize;
            let _meta = le_u32(&desc[16..20]);

            if payload_off + payload_len > pmem.len() {
                // malformed packet, drop.
            } else {
                let _payload = &pmem[payload_off..payload_off + payload_len];
                // Demo behavior: just "touch" payload + sock_id so optimizer doesn't delete.
                core::hint::black_box(sock_id);
                core::hint::black_box(_payload.len());
            }

            head = head.wrapping_add(1);
            consumed += 1;

            // Bound per notify call to avoid pathological loops.
            if consumed >= 64 {
                break;
            }
        }

        // Write back head. Tail is host-owned.
        write_le_u32(&mut dmem[12..16], head as u32);

        // Persist back desc buffer.
        DESC.with(|b| *b.borrow_mut() = dmem);
        consumed
    }
}

// Demo buffers. In a real shared-memory design these would be linear memories.
use core::cell::RefCell;
thread_local! {
    static DESC: RefCell<Vec<u8>> = RefCell::new(vec![0u8; 0x1000 + 32 * 128]);
    static PAYLOAD: RefCell<Vec<u8>> = RefCell::new(vec![0u8; 0x20000]);
}

export!(Component);
