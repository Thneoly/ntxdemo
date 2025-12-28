//! Shared-memory layout helpers used by the host.
//!
//! Current use (end-state pull model):
//! - The host *builds* `desc_mem` and `payload_mem` buffers in this layout.
//! - The buffers are enqueued into the host `rx-ring` provider.
//! - The guest pulls them via the `ntx:host/rx-ring@0.1.0` import and decodes
//!   the very same layout on the guest side.
//!
//! This module only defines the byte layout and encode/decode helpers.
//! It does **not** depend on any wasmtime types.

use crate::event_bus::Bytes;

pub const NTX_MAGIC: u32 = 0x4E54_5830; // "NTX0"
pub const NTX_VERSION: u16 = 1;

/// Fixed offsets used by the demo packet engine.
///
/// In multi-memory mode:
/// - `desc` memory uses `CONTROL_OFF` and `DESCS_OFF`.
/// - `payload` memory uses `PAYLOAD_OFF` (currently 0).
pub const CONTROL_OFF: u32 = 0x0000;
pub const DESCS_OFF: u32 = 0x1000;
pub const PAYLOAD_OFF: u32 = 0x0000;

/// Descriptor format (little endian), fixed-size and aligned.
///
/// Layout (bytes):
/// - 0..8   sock_id (u64)
/// - 8..12  payload_off (u32)
/// - 12..16 payload_len (u32)
/// - 16..20 meta (u32)
/// - 20..24 reserved (u32)
/// - 24..32 seq (u64)
pub const DESC_LEN: usize = 32;

/// Control block layout (little endian), fixed-size. v1 uses 48 bytes.
///
/// Layout:
/// - magic: u32
/// - version: u16
/// - flags: u16
/// - desc_capacity: u32
/// - desc_head: u32
/// - desc_tail: u32
/// - payload_capacity: u32
/// - payload_head: u32
/// - payload_tail: u32
/// - reserved: u32
pub const CONTROL_LEN: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlBlock {
    pub magic: u32,
    pub version: u16,
    pub flags: u16,
    pub desc_capacity: u32,
    pub desc_head: u32,
    pub desc_tail: u32,
    pub payload_capacity: u32,
    pub payload_head: u32,
    pub payload_tail: u32,
}

impl ControlBlock {
    pub fn new(desc_capacity: u32, payload_capacity: u32) -> Self {
        Self {
            magic: NTX_MAGIC,
            version: NTX_VERSION,
            flags: 0,
            desc_capacity,
            desc_head: 0,
            desc_tail: 0,
            payload_capacity,
            payload_head: 0,
            payload_tail: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    pub sock_id: u64,
    pub payload_off: u32,
    pub payload_len: u32,
    pub meta: u32,
    pub seq: u64,
}

impl Descriptor {
    pub fn rx(sock_id: Option<u64>, payload_off: u32, payload_len: u32, seq: u64) -> Self {
        const META_RX: u32 = 1;
        const META_HAS_SOCK: u32 = 1 << 1;
        let mut meta = META_RX;
        let sid = sock_id.unwrap_or(0);
        if sock_id.is_some() {
            meta |= META_HAS_SOCK;
        }
        Self {
            sock_id: sid,
            payload_off,
            payload_len,
            meta,
            seq,
        }
    }
}

pub fn encode_control(cb: &ControlBlock) -> [u8; CONTROL_LEN] {
    let mut b = [0u8; CONTROL_LEN];
    b[0..4].copy_from_slice(&cb.magic.to_le_bytes());
    b[4..6].copy_from_slice(&cb.version.to_le_bytes());
    b[6..8].copy_from_slice(&cb.flags.to_le_bytes());
    b[8..12].copy_from_slice(&cb.desc_capacity.to_le_bytes());
    b[12..16].copy_from_slice(&cb.desc_head.to_le_bytes());
    b[16..20].copy_from_slice(&cb.desc_tail.to_le_bytes());
    b[20..24].copy_from_slice(&cb.payload_capacity.to_le_bytes());
    b[24..28].copy_from_slice(&cb.payload_head.to_le_bytes());
    b[28..32].copy_from_slice(&cb.payload_tail.to_le_bytes());
    // 32..48 reserved
    b
}

pub fn decode_control(mem: &[u8]) -> Option<ControlBlock> {
    if mem.len() < (CONTROL_OFF as usize + CONTROL_LEN) {
        return None;
    }
    let off = CONTROL_OFF as usize;
    let b = &mem[off..off + CONTROL_LEN];

    let magic = u32::from_le_bytes(b[0..4].try_into().ok()?);
    let version = u16::from_le_bytes(b[4..6].try_into().ok()?);
    let flags = u16::from_le_bytes(b[6..8].try_into().ok()?);
    let desc_capacity = u32::from_le_bytes(b[8..12].try_into().ok()?);
    let desc_head = u32::from_le_bytes(b[12..16].try_into().ok()?);
    let desc_tail = u32::from_le_bytes(b[16..20].try_into().ok()?);
    let payload_capacity = u32::from_le_bytes(b[20..24].try_into().ok()?);
    let payload_head = u32::from_le_bytes(b[24..28].try_into().ok()?);
    let payload_tail = u32::from_le_bytes(b[28..32].try_into().ok()?);

    Some(ControlBlock {
        magic,
        version,
        flags,
        desc_capacity,
        desc_head,
        desc_tail,
        payload_capacity,
        payload_head,
        payload_tail,
    })
}

pub fn encode_desc(d: &Descriptor) -> [u8; DESC_LEN] {
    let mut b = [0u8; DESC_LEN];
    b[0..8].copy_from_slice(&d.sock_id.to_le_bytes());
    b[8..12].copy_from_slice(&d.payload_off.to_le_bytes());
    b[12..16].copy_from_slice(&d.payload_len.to_le_bytes());
    b[16..20].copy_from_slice(&d.meta.to_le_bytes());
    b[20..24].copy_from_slice(&0u32.to_le_bytes());
    b[24..32].copy_from_slice(&d.seq.to_le_bytes());
    b
}

pub fn decode_desc(b: &[u8]) -> Option<Descriptor> {
    if b.len() < DESC_LEN {
        return None;
    }
    let sock_id = u64::from_le_bytes(b[0..8].try_into().ok()?);
    let payload_off = u32::from_le_bytes(b[8..12].try_into().ok()?);
    let payload_len = u32::from_le_bytes(b[12..16].try_into().ok()?);
    let meta = u32::from_le_bytes(b[16..20].try_into().ok()?);
    let seq = u64::from_le_bytes(b[24..32].try_into().ok()?);
    Some(Descriptor {
        sock_id,
        payload_off,
        payload_len,
        meta,
        seq,
    })
}

/// A small helper to build a JSON string for the current demo path.
pub fn demo_json(sock_id: Option<u64>, payload: &[u8]) -> Bytes {
    let sid = sock_id.unwrap_or(0);
    let payload_hex = hex_encode(payload);
    let s = format!("{{\"sock\":{sid},\"payload_hex\":\"{payload_hex}\"}}\n");
    Bytes::from(s)
}

fn hex_encode(bytes: &[u8]) -> String {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(LUT[(b >> 4) as usize] as char);
        out.push(LUT[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_roundtrip() {
        let cb = ControlBlock::new(128, 65536);
        let enc = encode_control(&cb);
        let mut mem = vec![0u8; 0x2000];
        mem[CONTROL_OFF as usize..CONTROL_OFF as usize + CONTROL_LEN].copy_from_slice(&enc);
        let got = decode_control(&mem).unwrap();
        assert_eq!(got, cb);
    }

    #[test]
    fn desc_roundtrip() {
        let d = Descriptor::rx(Some(42), 0x4000, 12, 7);
        let enc = encode_desc(&d);
        let got = decode_desc(&enc).unwrap();
        assert_eq!(got, d);
    }
}
