use anyhow::bail;

use crate::network::Ipv4Addr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TcpFlags(pub u16);

impl TcpFlags {
    pub const FIN: u16 = 0x0001;
    pub const SYN: u16 = 0x0002;
    pub const RST: u16 = 0x0004;
    pub const PSH: u16 = 0x0008;
    pub const ACK: u16 = 0x0010;
    pub const URG: u16 = 0x0020;
    pub const ECE: u16 = 0x0040;
    pub const CWR: u16 = 0x0080;
    // NS lives in the IPv4 reserved bits / TCP header; we ignore it for now.

    pub fn contains(self, mask: u16) -> bool {
        (self.0 & mask) != 0
    }
}

/// Minimal TCP header (options are supported as raw bytes).
///
/// We keep this intentionally small: enough to do handshake + payload echo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub data_offset_words: u8,
    pub flags: TcpFlags,
    pub window_size: u16,
    pub urgent_ptr: u16,
    pub options: Vec<u8>,
}

impl TcpHeader {
    pub const MIN_LEN: usize = 20;

    pub fn header_len(&self) -> usize {
        (self.data_offset_words as usize) * 4
    }

    pub fn parse(pkt: &[u8]) -> anyhow::Result<(Self, &[u8])> {
        if pkt.len() < Self::MIN_LEN {
            bail!("tcp packet too short: {}", pkt.len());
        }

        let src_port = u16::from_be_bytes([pkt[0], pkt[1]]);
        let dst_port = u16::from_be_bytes([pkt[2], pkt[3]]);
        let seq = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
        let ack = u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]);

        let data_offset_words = pkt[12] >> 4;
        if data_offset_words < 5 {
            bail!("invalid tcp data offset: {}", data_offset_words);
        }
        let header_len = (data_offset_words as usize) * 4;
        if pkt.len() < header_len {
            bail!(
                "tcp header truncated: need {} have {}",
                header_len,
                pkt.len()
            );
        }

        let flags = TcpFlags(u16::from_be_bytes([pkt[12] & 0x0f, pkt[13]]));
        let window_size = u16::from_be_bytes([pkt[14], pkt[15]]);
        let urgent_ptr = u16::from_be_bytes([pkt[18], pkt[19]]);

        let options = if header_len > Self::MIN_LEN {
            pkt[Self::MIN_LEN..header_len].to_vec()
        } else {
            Vec::new()
        };

        Ok((
            TcpHeader {
                src_port,
                dst_port,
                seq,
                ack,
                data_offset_words,
                flags,
                window_size,
                urgent_ptr,
                options,
            },
            &pkt[header_len..],
        ))
    }

    pub fn write(
        &self,
        out: &mut [u8],
        payload: &[u8],
        src_ip: Ipv4Addr,
        dst_ip: Ipv4Addr,
    ) -> anyhow::Result<()> {
        let header_len = self.header_len();
        if header_len < Self::MIN_LEN {
            bail!("tcp header len too small: {}", header_len);
        }
        if header_len - Self::MIN_LEN != self.options.len() {
            bail!(
                "tcp options length mismatch: header_len={} options={}",
                header_len,
                self.options.len()
            );
        }
        if out.len() < header_len + payload.len() {
            bail!("buffer too small for tcp");
        }

        out[0..2].copy_from_slice(&self.src_port.to_be_bytes());
        out[2..4].copy_from_slice(&self.dst_port.to_be_bytes());
        out[4..8].copy_from_slice(&self.seq.to_be_bytes());
        out[8..12].copy_from_slice(&self.ack.to_be_bytes());

        out[12] = (self.data_offset_words << 4) | ((self.flags.0 >> 8) as u8 & 0x0f);
        out[13] = (self.flags.0 & 0xff) as u8;

        out[14..16].copy_from_slice(&self.window_size.to_be_bytes());
        out[16] = 0;
        out[17] = 0;
        out[18..20].copy_from_slice(&self.urgent_ptr.to_be_bytes());

        if !self.options.is_empty() {
            out[Self::MIN_LEN..header_len].copy_from_slice(&self.options);
        }

        out[header_len..header_len + payload.len()].copy_from_slice(payload);

        let csum = tcp_checksum(src_ip, dst_ip, &out[..header_len + payload.len()]);
        out[16..18].copy_from_slice(&csum.to_be_bytes());
        Ok(())
    }
}

/// TCP checksum (RFC 793) over pseudo-header + tcp header + payload.
pub fn tcp_checksum(src_ip: Ipv4Addr, dst_ip: Ipv4Addr, tcp_packet: &[u8]) -> u16 {
    let mut sum: u32 = 0;

    for chunk in src_ip.0.chunks_exact(2) {
        sum = sum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]) as u32);
    }
    for chunk in dst_ip.0.chunks_exact(2) {
        sum = sum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]) as u32);
    }

    // protocol + TCP length
    sum = sum.wrapping_add(0x0006);
    sum = sum.wrapping_add((tcp_packet.len() as u16) as u32);

    // TCP header + payload (checksum field considered zero)
    let mut i = 0;
    while i + 1 < tcp_packet.len() {
        if i == 16 {
            i += 2;
            continue;
        }
        let w = u16::from_be_bytes([tcp_packet[i], tcp_packet[i + 1]]) as u32;
        sum = sum.wrapping_add(w);
        i += 2;
    }
    if tcp_packet.len() % 2 == 1 {
        sum = sum.wrapping_add((tcp_packet[tcp_packet.len() - 1] as u32) << 8);
    }

    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }

    let csum = !(sum as u16);
    if csum == 0 { 0xffff } else { csum }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_checksum_roundtrip() {
        let src = Ipv4Addr([10, 0, 0, 1]);
        let dst = Ipv4Addr([10, 0, 0, 2]);
        let payload = b"hello";

        let hdr = TcpHeader {
            src_port: 1234,
            dst_port: 80,
            seq: 1,
            ack: 0,
            data_offset_words: 5,
            flags: TcpFlags(TcpFlags::SYN),
            window_size: 65535,
            urgent_ptr: 0,
            options: vec![],
        };

        let mut buf = vec![0u8; TcpHeader::MIN_LEN + payload.len()];
        hdr.write(&mut buf, payload, src, dst).unwrap();

        // Parse and verify fields survive.
        let (p, pl) = TcpHeader::parse(&buf).unwrap();
        assert_eq!(pl, payload);
        assert_eq!(p.src_port, 1234);
        assert_eq!(p.dst_port, 80);
        assert!(p.flags.contains(TcpFlags::SYN));

        // Recompute checksum; should match field.
        let got = u16::from_be_bytes([buf[16], buf[17]]);
        let expect = tcp_checksum(src, dst, &buf);
        assert_eq!(got, expect);
    }
}
