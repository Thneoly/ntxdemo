use crate::stack::{Layer, LayerId};
use crate::{Ipv4Addr, UdpHeader};

#[derive(Debug, Clone, Copy)]
pub struct Udp {
    pub src_port: u16,
    pub dst_port: u16,
    /// Optional: for checksum calculation when encoding.
    pub src_ip: Option<Ipv4Addr>,
    pub dst_ip: Option<Ipv4Addr>,
}

impl<'a> Layer<'a> for Udp {
    const ID: LayerId = LayerId::Udp;

    fn decode(data: &'a [u8]) -> Result<(Self, usize), String> {
        let (hdr, _payload) = UdpHeader::decode(data).map_err(|e| e.to_string())?;
        Ok((
            Self {
                src_port: hdr.src_port,
                dst_port: hdr.dst_port,
                src_ip: None,
                dst_ip: None,
            },
            UdpHeader::LEN,
        ))
    }

    fn encode(&self, payload: &[u8], out: &mut Vec<u8>) {
        out.clear();
        out.resize(UdpHeader::LEN + payload.len(), 0);

        let hdr = UdpHeader {
            src_port: self.src_port,
            dst_port: self.dst_port,
        };

        // If IPs are present, write with checksum; otherwise write with checksum=0.
        if let (Some(src), Some(dst)) = (self.src_ip, self.dst_ip) {
            let _ = hdr.encode(&mut out[..], payload, src, dst);
        } else {
            // Fallback: manually write header fields and length; checksum stays 0.
            out[0..2].copy_from_slice(&self.src_port.to_be_bytes());
            out[2..4].copy_from_slice(&self.dst_port.to_be_bytes());
            out[4..6].copy_from_slice(&((UdpHeader::LEN + payload.len()) as u16).to_be_bytes());
            out[6..8].copy_from_slice(&0u16.to_be_bytes());
            out[UdpHeader::LEN..].copy_from_slice(payload);
        }
    }

    fn next_layer(&self) -> Option<LayerId> {
        None
    }
}
