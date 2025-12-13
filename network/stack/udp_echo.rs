use crate::network::MacAddr;

use super::{Action, DecodedPacket, PacketHandler, build_udp_reply};

/// A minimal UDP echo handler.
///
/// Filter:
/// - Ethernet dst is iface mac OR broadcast
/// - IPv4 + UDP
/// - udp.dst_port == listen_port
pub struct UdpEchoHandler {
    pub listen_port: u16,
    pub iface_mac: MacAddr,
    pub verbose: bool,
}

impl PacketHandler for UdpEchoHandler {
    fn handle(&mut self, decoded: &DecodedPacket<'_>) -> anyhow::Result<Action> {
        // L2 filter.
        if !decoded.eth.dst.is_broadcast() && decoded.eth.dst != self.iface_mac {
            return Ok(Action::Pass);
        }

        // L3/L4 filter.
        let Some(ip) = decoded.ip else {
            return Ok(Action::Pass);
        };
        if ip.protocol != 17 {
            return Ok(Action::Pass);
        }
        let Some(udp) = decoded.udp else {
            return Ok(Action::Pass);
        };
        if udp.dst_port != self.listen_port {
            return Ok(Action::Pass);
        }

        if self.verbose {
            eprintln!(
                "echo hit: eth {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}; ip {}.{}.{}.{}:{} -> {}.{}.{}.{}:{}; payload_len={}",
                decoded.eth.src.0[0],
                decoded.eth.src.0[1],
                decoded.eth.src.0[2],
                decoded.eth.src.0[3],
                decoded.eth.src.0[4],
                decoded.eth.src.0[5],
                decoded.eth.dst.0[0],
                decoded.eth.dst.0[1],
                decoded.eth.dst.0[2],
                decoded.eth.dst.0[3],
                decoded.eth.dst.0[4],
                decoded.eth.dst.0[5],
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
                decoded.payload.len()
            );
        }

        Ok(Action::Reply(build_udp_reply(decoded, self.iface_mac)?))
    }
}
