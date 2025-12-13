use anyhow::bail;

use crate::network::Ipv4Addr;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
}

#[allow(dead_code)]
impl UdpHeader {
    pub const LEN: usize = 8;

    pub fn parse(pkt: &[u8]) -> anyhow::Result<(Self, &[u8])> {
        if pkt.len() < Self::LEN {
            bail!("udp packet too short: {}", pkt.len());
        }
        let src_port = u16::from_be_bytes([pkt[0], pkt[1]]);
        let dst_port = u16::from_be_bytes([pkt[2], pkt[3]]);
        let len = u16::from_be_bytes([pkt[4], pkt[5]]) as usize;
        if len < Self::LEN {
            bail!("invalid udp len={}", len);
        }
        if pkt.len() < len {
            bail!("udp truncated: len={} have {}", len, pkt.len());
        }
        Ok((UdpHeader { src_port, dst_port }, &pkt[Self::LEN..len]))
    }

    pub fn write(
        &self,
        out: &mut [u8],
        payload: &[u8],
        src_ip: Ipv4Addr,
        dst_ip: Ipv4Addr,
    ) -> anyhow::Result<()> {
        if out.len() < Self::LEN + payload.len() {
            bail!("buffer too small for udp");
        }
        out[0..2].copy_from_slice(&self.src_port.to_be_bytes());
        out[2..4].copy_from_slice(&self.dst_port.to_be_bytes());
        let len = (Self::LEN + payload.len()) as u16;
        out[4..6].copy_from_slice(&len.to_be_bytes());
        out[6] = 0;
        out[7] = 0;
        out[Self::LEN..Self::LEN + payload.len()].copy_from_slice(payload);

        let csum = udp_checksum(src_ip, dst_ip, &out[..Self::LEN + payload.len()]);
        // RFC768: checksum of 0 means "not used". In IPv4 it's allowed.
        // We still compute it for better interoperability.
        out[6..8].copy_from_slice(&csum.to_be_bytes());
        Ok(())
    }
}

#[allow(dead_code)]
pub fn udp_checksum(src_ip: Ipv4Addr, dst_ip: Ipv4Addr, udp_packet: &[u8]) -> u16 {
    // Pseudo header + UDP header + payload.
    let mut sum: u32 = 0;

    // src/dst
    for chunk in src_ip.0.chunks_exact(2) {
        sum = sum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]) as u32);
    }
    for chunk in dst_ip.0.chunks_exact(2) {
        sum = sum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]) as u32);
    }

    // protocol + UDP length
    sum = sum.wrapping_add(0x0011); // protocol UDP (17)
    sum = sum.wrapping_add((udp_packet.len() as u16) as u32);

    // UDP header + payload (checksum field considered zero)
    let mut i = 0;
    while i + 1 < udp_packet.len() {
        if i == 6 {
            i += 2;
            continue;
        }
        let w = u16::from_be_bytes([udp_packet[i], udp_packet[i + 1]]) as u32;
        sum = sum.wrapping_add(w);
        i += 2;
    }
    if udp_packet.len() % 2 == 1 {
        sum = sum.wrapping_add((udp_packet[udp_packet.len() - 1] as u32) << 8);
    }

    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let csum = !(sum as u16);
    if csum == 0 { 0xffff } else { csum }
}
