use crate::stack::{AcceptResult, Layer, LayerId, PacketContext};
use crate::{Ipv4Addr, Ipv4Header};

#[derive(Debug, Clone, Copy)]
pub struct Ipv4 {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub proto: u8,
    pub ttl: u8,
    pub identification: u16,
    pub flags_fragment: u16,
    pub ihl_bytes: usize,
}

impl<'a> Layer<'a> for Ipv4 {
    const ID: LayerId = LayerId::Ipv4;

    fn decode(data: &'a [u8]) -> Result<(Self, usize), String> {
        if data.len() < Ipv4Header::MIN_LEN {
            return Err("short ipv4".into());
        }
        let version_ihl = data[0];
        let version = version_ihl >> 4;
        if version != 4 {
            return Err("not ipv4".into());
        }
        let ihl_words = (version_ihl & 0x0f) as usize;
        let ihl_bytes = ihl_words * 4;
        if ihl_words < 5 {
            return Err("invalid ihl".into());
        }
        if data.len() < ihl_bytes {
            return Err("truncated ipv4 header".into());
        }

        let (hdr, _payload) = Ipv4Header::decode(data).map_err(|e| e.to_string())?;

        Ok((
            Self {
                src: hdr.src,
                dst: hdr.dst,
                proto: hdr.protocol,
                ttl: hdr.ttl,
                identification: hdr.identification,
                flags_fragment: hdr.flags_fragment,
                ihl_bytes,
            },
            ihl_bytes,
        ))
    }

    fn encode(&self, payload: &[u8], out: &mut Vec<u8>) {
        // Encode minimal IPv4 header (no options).
        let ip_len = Ipv4Header::MIN_LEN;
        out.clear();
        out.resize(ip_len + payload.len(), 0);
        let hdr = Ipv4Header {
            src: self.src,
            dst: self.dst,
            protocol: self.proto,
            ttl: self.ttl,
            identification: self.identification,
            flags_fragment: self.flags_fragment,
        };
        let _ = hdr.encode(&mut out[..ip_len], payload.len(), 0);
        out[ip_len..].copy_from_slice(payload);
    }

    fn accept(&self, ctx: &PacketContext) -> AcceptResult {
        // Prefer ABR (Active Binding Resource) view as the single source of truth.
        if let Some(view) = ctx.abr.as_ref() {
            // Convention: ABR stores IPv4 as big-endian u32.
            let dst_be = u32::from_be_bytes(self.dst.octets());
            if view.ipv4.contains_be(dst_be) {
                return AcceptResult::Accept;
            }
            // Not bound to us.
            return AcceptResult::Poison;
        }

        // Back-compat fallback: if caller didn't configure local IPv4 ownership, accept all.
        if ctx.local_ipv4.is_empty() {
            return AcceptResult::Accept;
        }

        if ctx.local_ipv4.iter().any(|ip| *ip == self.dst) {
            AcceptResult::Accept
        } else {
            AcceptResult::Poison
        }
    }

    fn next_layer(&self) -> Option<LayerId> {
        match self.proto {
            17 => Some(LayerId::Udp),
            6 => Some(LayerId::Tcp),
            _ => Some(LayerId::Payload),
        }
    }
}
