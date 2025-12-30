//! Small binary decoding helpers (little-endian) and formatting utilities.

#[inline]
pub fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[inline]
pub fn le_u16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

#[inline]
pub fn le_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

pub fn to_hex(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        // two lowercase hex digits
        out.push(hex_digit(b >> 4));
        out.push(hex_digit(b & 0x0f));
    }
    out
}

#[inline]
fn hex_digit(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        10..=15 => (b'a' + (v - 10)) as char,
        _ => '?',
    }
}
