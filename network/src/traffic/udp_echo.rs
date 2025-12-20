use crate::{ETH_TYPE_IPV4, EthernetHeader, Ipv4Header, MacAddr, UdpHeader};

use crate::packet::layers::{Ether, Ipv4, Udp};
use crate::socket::PacketView;
use crate::stack::{Action, PacketHandler, ParsedPacket, ReplyFrame};

/// A minimal UDP echo handler.
///
/// Filter:
/// - Ethernet dst is iface mac OR broadcast
/// - IPv4 + UDP
/// - udp.dst_port == listen_port
#[derive(Debug, Clone)]
pub struct UdpEchoHandler {
    pub listen_port: u16,
    pub iface_mac: MacAddr,
    pub verbose: bool,
}

impl PacketHandler for UdpEchoHandler {
    fn handle(&mut self, pkt: &ParsedPacket<'_>) -> anyhow::Result<Action> {
        let Some(eth) = pkt.get::<Ether>() else {
            return Ok(Action::Pass);
        };

        // L2 filter
        if !eth.dst.is_broadcast() && eth.dst != self.iface_mac {
            return Ok(Action::Pass);
        }

        let Some(ip) = pkt.get::<Ipv4>() else {
            return Ok(Action::Pass);
        };
        if ip.proto != 17 {
            return Ok(Action::Pass);
        }

        let Some(udp) = pkt.get::<Udp>() else {
            return Ok(Action::Pass);
        };
        if udp.dst_port != self.listen_port {
            return Ok(Action::Pass);
        }

        if self.verbose {
            eprintln!(
                "echo hit: eth {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}; ip {}.{}.{}.{}:{} -> {}.{}.{}.{}:{}; payload_len={}",
                eth.src.0[0],
                eth.src.0[1],
                eth.src.0[2],
                eth.src.0[3],
                eth.src.0[4],
                eth.src.0[5],
                eth.dst.0[0],
                eth.dst.0[1],
                eth.dst.0[2],
                eth.dst.0[3],
                eth.dst.0[4],
                eth.dst.0[5],
                ip.src.0[0],
                ip.src.0[1],
                ip.src.0[2],
                ip.src.0[3],
                udp.src_port,
                ip.dst.0[0],
                ip.dst.0[1],
                ip.dst.0[2],
                ip.dst.0[3],
                udp.dst_port,
                pkt.payload().len()
            );
        }

        Ok(Action::Reply(build_udp_reply(pkt, self.iface_mac)?))
    }
}

/// Build an IPv4/UDP echo reply, swapping MAC/IP/port.
///
/// Requires the parsed packet to contain Ether + Ipv4 + Udp.
pub fn build_udp_reply(pkt: &impl PacketView, iface_mac: MacAddr) -> anyhow::Result<ReplyFrame> {
    let eth = pkt
        .get::<Ether>()
        .ok_or_else(|| anyhow::anyhow!("missing ether"))?;
    let ip = pkt
        .get::<Ipv4>()
        .ok_or_else(|| anyhow::anyhow!("missing ipv4"))?;
    let udp = pkt
        .get::<Udp>()
        .ok_or_else(|| anyhow::anyhow!("missing udp"))?;

    let payload = pkt
        .payload()
        .ok_or_else(|| anyhow::anyhow!("missing payload"))?;

    let reply_eth = EthernetHeader {
        dst: eth.src,
        src: iface_mac,
        ethertype: ETH_TYPE_IPV4,
    };

    let reply_ip = Ipv4Header {
        src: ip.dst,
        dst: ip.src,
        protocol: 17,
        ttl: ip.ttl,
        identification: ip.identification,
        flags_fragment: ip.flags_fragment,
    };

    let reply_udp = UdpHeader {
        src_port: udp.dst_port,
        dst_port: udp.src_port,
    };

    let eth_len = EthernetHeader::LEN;
    let ip_len = Ipv4Header::MIN_LEN;
    let udp_len = UdpHeader::LEN;

    let mut bytes = vec![0u8; eth_len + ip_len + udp_len + payload.len()];

    reply_eth.encode(&mut bytes[..eth_len])?;
    reply_ip.encode(
        &mut bytes[eth_len..eth_len + ip_len],
        udp_len + payload.len(),
        0,
    )?;

    let udp_off = eth_len + ip_len;
    reply_udp.encode(
        &mut bytes[udp_off..udp_off + udp_len + payload.len()],
        payload,
        reply_ip.src,
        reply_ip.dst,
    )?;

    Ok(ReplyFrame { bytes })
}

/// A reusable, socket-like template for replying to a specific UDP flow.
///
/// Intended usage:
/// - Build once from a received packet (`UdpReplyTemplate::from_parsed_packet`).
/// - Reuse for subsequent replies that should go back along the same L2/L3/L4 route,
///   only changing the payload.
#[derive(Debug, Clone, Copy)]
pub struct UdpReplyTemplate {
    pub eth: EthernetHeader,
    pub ip: Ipv4Header,
    pub udp: UdpHeader,
}

impl UdpReplyTemplate {
    /// Create a reply template by swapping src/dst of Ether/IPv4/UDP.
    ///
    /// `src_mac` is the MAC to use for the reply's Ethernet source.
    pub fn from_layers(eth: &Ether, ip: &Ipv4, udp: &Udp, src_mac: MacAddr) -> Self {
        Self {
            eth: EthernetHeader {
                dst: eth.src,
                src: src_mac,
                ethertype: ETH_TYPE_IPV4,
            },
            ip: Ipv4Header {
                src: ip.dst,
                dst: ip.src,
                protocol: 17,
                ttl: ip.ttl,
                identification: ip.identification,
                flags_fragment: ip.flags_fragment,
            },
            udp: UdpHeader {
                src_port: udp.dst_port,
                dst_port: udp.src_port,
            },
        }
    }

    /// Create a reply template from a parsed packet.
    ///
    /// Errors if the packet does not contain Ether + Ipv4 + Udp.
    pub fn from_parsed_packet(pkt: &impl PacketView, src_mac: MacAddr) -> anyhow::Result<Self> {
        let eth = pkt
            .get::<Ether>()
            .ok_or_else(|| anyhow::anyhow!("missing ether"))?;
        let ip = pkt
            .get::<Ipv4>()
            .ok_or_else(|| anyhow::anyhow!("missing ipv4"))?;
        let udp = pkt
            .get::<Udp>()
            .ok_or_else(|| anyhow::anyhow!("missing udp"))?;
        Ok(Self::from_layers(eth, ip, udp, src_mac))
    }

    /// Build a reply frame for `payload`.
    ///
    /// This always computes IPv4 header checksum and UDP checksum.
    pub fn build(self, payload: &[u8]) -> anyhow::Result<ReplyFrame> {
        let eth_len = EthernetHeader::LEN;
        let ip_len = Ipv4Header::MIN_LEN;
        let udp_len = UdpHeader::LEN;

        let mut bytes = vec![0u8; eth_len + ip_len + udp_len + payload.len()];

        self.eth.encode(&mut bytes[..eth_len])?;
        self.ip.encode(
            &mut bytes[eth_len..eth_len + ip_len],
            udp_len + payload.len(),
            0,
        )?;

        let udp_off = eth_len + ip_len;
        self.udp.encode(
            &mut bytes[udp_off..udp_off + udp_len + payload.len()],
            payload,
            self.ip.src,
            self.ip.dst,
        )?;

        Ok(ReplyFrame { bytes })
    }
}
