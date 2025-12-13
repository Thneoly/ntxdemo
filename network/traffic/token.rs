use anyhow::bail;

/// A tiny request token embedded into UDP payload.
///
/// Format (network byte order):
///
/// - 4 bytes magic: "NTX1" (0x4e 0x54 0x58 0x31)
/// - 8 bytes seq: u64 big-endian
///
/// Total: 12 bytes.
pub const TOKEN_LEN: usize = 12;
const MAGIC: [u8; 4] = *b"NTX1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token(pub u64);

pub fn encode_token(seq: u64) -> [u8; TOKEN_LEN] {
    let mut b = [0u8; TOKEN_LEN];
    b[0..4].copy_from_slice(&MAGIC);
    b[4..12].copy_from_slice(&seq.to_be_bytes());
    b
}

pub fn decode_token(payload: &[u8]) -> anyhow::Result<Token> {
    if payload.len() < TOKEN_LEN {
        bail!("payload too short for token: {}", payload.len());
    }
    if payload[0..4] != MAGIC {
        bail!("token magic mismatch");
    }
    let mut s = [0u8; 8];
    s.copy_from_slice(&payload[4..12]);
    Ok(Token(u64::from_be_bytes(s)))
}
