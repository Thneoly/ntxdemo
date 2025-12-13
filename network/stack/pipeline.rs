use crate::network::{ETH_TYPE_IPV4, EthernetHeader, Ipv4Header, MacAddr, UdpHeader};

use super::{DecodedPacket, PacketContext};

/// A reply frame buffer.
#[derive(Debug, Clone)]
pub struct ReplyFrame {
    pub bytes: Vec<u8>,
}

/// The result of a handler.
#[derive(Debug, Clone)]
pub enum Action {
    /// Ignore this packet.
    Pass,
    /// Send a reply frame.
    Reply(ReplyFrame),
}

/// Stateless packet handler.
pub trait PacketHandler {
    fn handle(&mut self, decoded: &DecodedPacket<'_>) -> anyhow::Result<Action>;
}

/// Helper to build an IPv4/UDP echo reply, swapping MAC/IP/port.
///
/// Assumes the input is Ethernet + IPv4 + UDP.
pub fn build_udp_reply(
    decoded: &DecodedPacket<'_>,
    iface_mac: MacAddr,
) -> anyhow::Result<ReplyFrame> {
    let ip = decoded.ip.ok_or_else(|| anyhow::anyhow!("missing ipv4"))?;
    let udp = decoded.udp.ok_or_else(|| anyhow::anyhow!("missing udp"))?;

    let payload = decoded.payload;

    let reply_eth = EthernetHeader {
        dst: decoded.eth.src,
        src: iface_mac,
        ethertype: ETH_TYPE_IPV4,
    };

    let reply_ip = Ipv4Header {
        src: ip.dst,
        dst: ip.src,
        protocol: 17,
        ttl: 64,
        identification: 0,
        flags_fragment: 0,
    };

    let reply_udp = UdpHeader {
        src_port: udp.dst_port,
        dst_port: udp.src_port,
    };

    let eth_len = EthernetHeader::LEN;
    let ip_len = Ipv4Header::MIN_LEN;
    let udp_len = UdpHeader::LEN;

    let mut bytes = vec![0u8; eth_len + ip_len + udp_len + payload.len()];

    reply_eth.write(&mut bytes[..eth_len])?;
    reply_ip.write(
        &mut bytes[eth_len..eth_len + ip_len],
        udp_len + payload.len(),
        0,
    )?;

    let udp_off = eth_len + ip_len;
    reply_udp.write(
        &mut bytes[udp_off..udp_off + udp_len + payload.len()],
        payload,
        reply_ip.src,
        reply_ip.dst,
    )?;

    Ok(ReplyFrame { bytes })
}

/// A small orchestrator that decodes a frame and runs a set of handlers.
///
/// The first handler returning `Action::Reply` wins.
pub struct Pipeline {
    handlers: Vec<Box<dyn PacketHandler>>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub fn add_handler<H: PacketHandler + 'static>(&mut self, h: H) {
        self.handlers.push(Box::new(h));
    }

    pub fn process(&mut self, ctx: &PacketContext) -> anyhow::Result<Action> {
        let decoded = ctx.decode()?;
        for h in self.handlers.iter_mut() {
            match h.handle(&decoded)? {
                Action::Pass => continue,
                r @ Action::Reply(_) => return Ok(r),
            }
        }
        Ok(Action::Pass)
    }
}
