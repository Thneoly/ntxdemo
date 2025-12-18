use crate::stack::{AcceptResult, Layer, LayerId, PacketContext};
use crate::{ArpPacket, ETH_TYPE_ARP, EthernetHeader, MacAddr};

/// ARP layer.
///
/// This is a thin wrapper around [`crate::packet::headers::ArpPacket`].
#[derive(Debug, Clone, Copy)]
pub struct Arp {
    pub oper: u16,
    pub sha: MacAddr,
    pub spa: crate::Ipv4Addr,
    pub tha: MacAddr,
    pub tpa: crate::Ipv4Addr,
}

impl From<ArpPacket> for Arp {
    fn from(p: ArpPacket) -> Self {
        Self {
            oper: p.oper,
            sha: p.sha,
            spa: p.spa,
            tha: p.tha,
            tpa: p.tpa,
        }
    }
}

impl From<Arp> for ArpPacket {
    fn from(a: Arp) -> Self {
        Self {
            oper: a.oper,
            sha: a.sha,
            spa: a.spa,
            tha: a.tha,
            tpa: a.tpa,
        }
    }
}

impl<'a> Layer<'a> for Arp {
    const ID: LayerId = LayerId::Arp;

    fn decode(input: &'a [u8]) -> Result<(Self, usize), String> {
        // ARP sits directly in Ethernet payload.
        let p = ArpPacket::decode(input).map_err(|e| e.to_string())?;
        Ok((p.into(), ArpPacket::LEN))
    }

    fn encode(&self, _payload: &[u8], out: &mut Vec<u8>) {
        out.clear();
        out.resize(ArpPacket::LEN, 0);
        let p: ArpPacket = (*self).into();
        // Safe to ignore: we sized the buffer correctly.
        let _ = p.encode(&mut out[..]);
    }

    fn accept(&self, ctx: &PacketContext) -> AcceptResult {
        // If ABR is present, only accept ARP that targets an active/bound local IP.
        // Otherwise fall back to permissive behavior.
        let Some(view) = ctx.abr.as_ref() else {
            return AcceptResult::Accept;
        };

        let tpa_be = u32::from_be_bytes(self.tpa.octets());
        if view.ipv4.contains_be(tpa_be) {
            AcceptResult::Accept
        } else {
            // Valid ARP, but not for our IP resource.
            AcceptResult::Poison
        }
    }

    fn next_layer(&self) -> Option<LayerId> {
        None
    }
}

/// Helper: build an Ethernet+ARP frame from an [`Arp`] layer.
///
/// This is convenient when you want to emit ARP without going through the generic
/// `build_packet` loop.
#[allow(dead_code)]
pub fn build_ether_arp_frame(eth: EthernetHeader, arp: Arp) -> Result<Vec<u8>, String> {
    if eth.ethertype != ETH_TYPE_ARP {
        return Err("eth.ethertype must be ETH_TYPE_ARP".into());
    }
    let mut out = vec![0u8; EthernetHeader::LEN + ArpPacket::LEN];
    eth.encode(&mut out[..EthernetHeader::LEN])
        .map_err(|e| e.to_string())?;
    let p: ArpPacket = arp.into();
    p.encode(&mut out[EthernetHeader::LEN..])
        .map_err(|e| e.to_string())?;
    Ok(out)
}
