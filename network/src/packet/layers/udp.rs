use crate::stack::{AcceptResult, Layer, LayerId, PacketContext};
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

    fn accept(&self, ctx: &PacketContext) -> AcceptResult {
        let Some(view) = ctx.abr.as_ref() else {
            return AcceptResult::Accept;
        };

        // Require dst_ip to be known (filled by build glue / previous layers) to apply
        // ip+port binding. If not known, be permissive.
        let Some(dst_ip) = self.dst_ip else {
            return AcceptResult::Accept;
        };
        let dst_ip_be = u32::from_be_bytes(dst_ip.octets());

        // Policy:
        // - exact (dst_ip, dst_port) bound => Accept
        // - wildcard ip (0.0.0.0, dst_port) bound => Accept
        // - else => Poison (valid UDP but not for our bound ports)
        if view.udp_ports.contains_be(dst_ip_be, self.dst_port)
            || view.udp_ports.contains_be(0, self.dst_port)
        {
            AcceptResult::Accept
        } else {
            AcceptResult::Poison
        }
    }

    fn next_layer(&self) -> Option<LayerId> {
        None
    }
}
