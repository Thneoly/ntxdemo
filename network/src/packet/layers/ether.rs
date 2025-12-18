use crate::stack::{AcceptResult, Layer, LayerId, PacketContext};
use crate::{ETH_TYPE_ARP, ETH_TYPE_IPV4, EthernetHeader, MacAddr};

#[derive(Debug, Clone, Copy)]
pub struct Ether {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ethertype: u16,
}

impl<'a> Layer<'a> for Ether {
    const ID: LayerId = LayerId::Ether;

    fn decode(data: &'a [u8]) -> Result<(Self, usize), String> {
        let (hdr, _rest) = EthernetHeader::decode(data).map_err(|e| e.to_string())?;
        Ok((
            Self {
                dst: hdr.dst,
                src: hdr.src,
                ethertype: hdr.ethertype,
            },
            EthernetHeader::LEN,
        ))
    }

    fn encode(&self, payload: &[u8], out: &mut Vec<u8>) {
        out.clear();
        out.resize(EthernetHeader::LEN + payload.len(), 0);
        let hdr = EthernetHeader {
            dst: self.dst,
            src: self.src,
            ethertype: self.ethertype,
        };
        // Safe to unwrap: buffer sizes are correct
        let _ = hdr.encode(&mut out[..EthernetHeader::LEN]);
        out[EthernetHeader::LEN..].copy_from_slice(payload);
    }

    fn accept(&self, ctx: &PacketContext) -> AcceptResult {
        let Some(local) = ctx.iface_mac else {
            return AcceptResult::Accept;
        };

        // Accept unicast to us, broadcast, and multicast.
        if self.dst == local {
            return AcceptResult::Accept;
        }
        if self.dst.0 == [0xff; 6] {
            return AcceptResult::Accept;
        }
        if (self.dst.0[0] & 0x01) == 0x01 {
            return AcceptResult::Accept;
        }
        AcceptResult::Drop
    }

    fn next_layer(&self) -> Option<LayerId> {
        match self.ethertype {
            ETH_TYPE_IPV4 => Some(LayerId::Ipv4),
            ETH_TYPE_ARP => Some(LayerId::Arp),
            _ => None,
        }
    }
}
