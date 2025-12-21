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

/// Try to extract UDP application payload from an Ethernet/IPv4/UDP frame.
///
/// Current host RX path enqueues full frames (L2+) into the shared payload buffer.
/// If parsing fails, returns `None` and the caller may drop the packet.
fn udp_app_payload(frame: &[u8]) -> Option<&[u8]> {
    // Ethernet header: 14 bytes
    if frame.len() < 14 {
        return None;
    }
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    // IPv4 ethertype
    if ethertype != 0x0800 {
        return None;
    }

    let ip_off = 14;
    if frame.len() < ip_off + 20 {
        return None;
    }
    let ver_ihl = frame[ip_off];
    let version = ver_ihl >> 4;
    if version != 4 {
        return None;
    }
    let ihl_words = (ver_ihl & 0x0f) as usize;
    let ihl = ihl_words * 4;
    if ihl < 20 {
        return None;
    }
    if frame.len() < ip_off + ihl {
        return None;
    }

    let total_len = u16::from_be_bytes([frame[ip_off + 2], frame[ip_off + 3]]) as usize;
    if total_len < ihl {
        return None;
    }
    // Constrain to what we actually have.
    let ip_end = (ip_off + total_len).min(frame.len());

    let proto = frame[ip_off + 9];
    // UDP protocol
    if proto != 17 {
        return None;
    }

    let udp_off = ip_off + ihl;
    if ip_end < udp_off + 8 {
        return None;
    }
    let udp_len = u16::from_be_bytes([frame[udp_off + 4], frame[udp_off + 5]]) as usize;
    if udp_len < 8 {
        return None;
    }
    let payload_off = udp_off + 8;
    let payload_end = (udp_off + udp_len).min(ip_end);
    if payload_end < payload_off {
        return None;
    }
    Some(&frame[payload_off..payload_end])
}

fn handle_udp_datagram(sock_id: u64, payload: &[u8]) {
    // Demo behavior: echo only application-layer payload back via hostnet.
    // Best-effort parsing: if we can't parse UDP, drop.
    let Some(app) = udp_app_payload(payload) else {
        return;
    };

    // Errors are best-effort; on failure we just drop.
    if let Ok(frame) = ntx::hostnet::udp_socket_control::build_reply(sock_id, app) {
        let _ = ntx::hostnet::udp_socket_control::tx(frame);
    }
}

/// Drain host-provided RX descriptors and dispatch them to `handle_udp_datagram`.
///
/// Returns (new_head, consumed).
fn drain_rx_ring(desc_mem: &mut [u8], payload_mem: &[u8]) -> (usize, u32) {
    if desc_mem.len() < CONTROL_LEN {
        return (0, 0);
    }

    // Parse control block at 0.
    let magic = le_u32(&desc_mem[0..4]);
    let version = le_u16(&desc_mem[4..6]);
    if magic != NTX_MAGIC || version != NTX_VERSION {
        return (0, 0);
    }

    let desc_capacity = le_u32(&desc_mem[8..12]) as usize;
    let mut head = le_u32(&desc_mem[12..16]) as usize;
    let tail = le_u32(&desc_mem[16..20]) as usize;

    // Ring starts at offset 0x1000 in desc memory (keep consistent with host v1).
    let descs_off: usize = 0x1000;

    if desc_capacity == 0 {
        return (head, 0);
    }

    let mut consumed: u32 = 0;
    // Host enqueues by advancing `desc_tail`; guest consumes by advancing `desc_head`.
    while head != tail {
        let idx = head % desc_capacity;
        let base = descs_off + idx * DESC_LEN;
        if base + DESC_LEN > desc_mem.len() {
            break;
        }

        let desc = &desc_mem[base..base + DESC_LEN];
        let sock_id = le_u64(&desc[0..8]);
        let payload_off = le_u32(&desc[8..12]) as usize;
        let payload_len = le_u32(&desc[12..16]) as usize;
        let _meta = le_u32(&desc[16..20]);

        if payload_off + payload_len <= payload_mem.len() {
            let payload = &payload_mem[payload_off..payload_off + payload_len];
            handle_udp_datagram(sock_id, payload);
        }

        head = head.wrapping_add(1);
        consumed += 1;

        // Bound per notify call to avoid pathological loops.
        if consumed >= 64 {
            break;
        }
    }

    (head, consumed)
}

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

        let (new_head, consumed) = drain_rx_ring(&mut dmem, &pmem);

        if dmem.len() >= 16 {
            // Write back head. Tail is host-owned.
            write_le_u32(&mut dmem[12..16], new_head as u32);
        }

        // Persist back desc buffer.
        DESC.with(|b| *b.borrow_mut() = dmem);
        consumed
    }

    fn run() -> Result<(), String> {
        // Minimal closed-loop TX demo driven from the guest.
        // We keep values explicit to avoid depending on environment variables.
        let owner = ntx::hostnet::resources::create_socket_owner("packet-engine")
            .map_err(|e| format!("create_socket_owner failed: {e:?}"))?;

        // Acquire+pin local ipv4/mac/udp-port (host chooses actual values).
        ntx::hostnet::resources::acquire_udp_port("default", &owner)
            .map_err(|e| format!("acquire_udp_port failed: {e:?}"))?;

        // Create UDP socket id.
        let sock = ntx::hostnet::udp_socket_control::create("echo")
            .map_err(|e| format!("udp.create failed: {e:?}"))?;

        // Demo peer tuple.
        // NOTE: This assumes your host test topology has a reachable peer at 10.0.0.2.
        let peer_ipv4 = ntx::hostnet::types::Ipv4Addr {
            a: 10,
            b: 0,
            c: 0,
            d: 2,
        };
        let peer_port: u16 = 7;
        let peer_mac = ntx::hostnet::types::MacAddr {
            a: 2,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            f: 2,
        };

        // Local identity: for now we use stable demo values.
        // Ownership correctness is enforced host-side by mapping value->rid and checking owner.
        let local_ipv4 = ntx::hostnet::types::Ipv4Addr {
            a: 10,
            b: 0,
            c: 0,
            d: 1,
        };
        let local_mac = ntx::hostnet::types::MacAddr {
            a: 2,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            f: 1,
        };

        // Pick a local UDP port by resolving a rid from the acquired pool.
        // The minimal host resource API currently only provides `resolve_udp_port(rid)`;
        // since `acquire_udp_port` doesn't return the rid, we use a conventional demo port.
        // Host will map it to the pinned resource for this owner.
        let local_udp_port: u16 = 10000;

        let bind = ntx::hostnet::udp_socket_control::UdpBind {
            local_ipv4,
            local_mac,
            local_udp_port,
            peer_ipv4,
            peer_port,
            peer_mac,
            ttl: Some(64),
        };

        ntx::hostnet::udp_socket_control::bind(sock.sock, bind)
            .map_err(|e| format!("udp.bind failed: {e:?}"))?;

        // Send one application payload (no headers).
        let payload: &[u8] = b"hello from guest";
        let frame = ntx::hostnet::udp_socket_control::build_reply(sock.sock, payload)
            .map_err(|e| format!("udp.build_reply failed: {e:?}"))?;
        let _ = ntx::hostnet::udp_socket_control::tx(frame)
            .map_err(|e| format!("udp.tx failed: {e:?}"))?;
        Ok(())
    }
}

// Demo buffers. In a real shared-memory design these would be linear memories.
use core::cell::RefCell;
thread_local! {
    static DESC: RefCell<Vec<u8>> = RefCell::new(vec![0u8; 0x1000 + 32 * 128]);
    static PAYLOAD: RefCell<Vec<u8>> = RefCell::new(vec![0u8; 0x20000]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udp_app_payload_returns_udp_body() {
        // Build a minimal Ethernet + IPv4 + UDP packet.
        let app = b"hello";

        let mut frame = Vec::new();
        // Ethernet
        frame.extend_from_slice(&[0u8; 6]); // dst
        frame.extend_from_slice(&[0u8; 6]); // src
        frame.extend_from_slice(&0x0800u16.to_be_bytes()); // ethertype ipv4

        // IPv4 header (20 bytes)
        let ver_ihl = (4u8 << 4) | 5u8; // v4, IHL=5
        frame.push(ver_ihl);
        frame.push(0); // dscp/ecn
        let total_len = (20 + 8 + app.len()) as u16;
        frame.extend_from_slice(&total_len.to_be_bytes());
        frame.extend_from_slice(&0u16.to_be_bytes()); // identification
        frame.extend_from_slice(&0u16.to_be_bytes()); // flags/fragment
        frame.push(64); // ttl
        frame.push(17); // proto udp
        frame.extend_from_slice(&0u16.to_be_bytes()); // checksum (ignored)
        frame.extend_from_slice(&[10, 0, 0, 1]); // src
        frame.extend_from_slice(&[10, 0, 0, 2]); // dst

        // UDP header
        frame.extend_from_slice(&1234u16.to_be_bytes());
        frame.extend_from_slice(&5678u16.to_be_bytes());
        let udp_len = (8 + app.len()) as u16;
        frame.extend_from_slice(&udp_len.to_be_bytes());
        frame.extend_from_slice(&0u16.to_be_bytes()); // checksum

        // payload
        frame.extend_from_slice(app);

        let got = udp_app_payload(&frame).unwrap();
        assert_eq!(got, app);
    }

    #[test]
    fn udp_app_payload_short_is_none() {
        assert!(udp_app_payload(&[0u8; 10]).is_none());
    }
}

export!(Component);
