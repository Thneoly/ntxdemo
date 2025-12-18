use anyhow::bail;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    #[allow(dead_code)]
    pub fn octets(&self) -> [u8; 4] {
        self.0
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct Ipv4Header {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub protocol: u8,
    pub ttl: u8,
    pub identification: u16,
    pub flags_fragment: u16,
}

#[allow(dead_code)]
impl Ipv4Header {
    pub const MIN_LEN: usize = 20;

    /// Decode an IPv4 header from `pkt`.
    ///
    /// Returns `(header, payload_slice)` where payload length is derived from total_len.
    pub fn decode(pkt: &[u8]) -> anyhow::Result<(Self, &[u8])> {
        if pkt.len() < Self::MIN_LEN {
            bail!("ipv4 packet too short: {}", pkt.len());
        }
        let version_ihl = pkt[0];
        let version = version_ihl >> 4;
        let ihl = (version_ihl & 0x0f) as usize;
        if version != 4 {
            bail!("not ipv4: version={}", version);
        }
        if ihl < 5 {
            bail!("invalid ihl={}", ihl);
        }
        let header_len = ihl * 4;
        if pkt.len() < header_len {
            bail!(
                "ipv4 header truncated: need {} have {}",
                header_len,
                pkt.len()
            );
        }

        let total_len = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
        if total_len < header_len {
            bail!("invalid total_len={}", total_len);
        }
        if pkt.len() < total_len {
            bail!(
                "ipv4 packet truncated: total_len={} have {}",
                total_len,
                pkt.len()
            );
        }

        let protocol = pkt[9];
        let ttl = pkt[8];
        let identification = u16::from_be_bytes([pkt[4], pkt[5]]);
        let flags_fragment = u16::from_be_bytes([pkt[6], pkt[7]]);
        let src = Ipv4Addr(pkt[12..16].try_into().unwrap());
        let dst = Ipv4Addr(pkt[16..20].try_into().unwrap());

        Ok((
            Ipv4Header {
                src,
                dst,
                protocol,
                ttl,
                identification,
                flags_fragment,
            },
            &pkt[header_len..total_len],
        ))
    }

    pub fn encode(&self, out: &mut [u8], payload_len: usize, dscp_ecn: u8) -> anyhow::Result<()> {
        if out.len() < Self::MIN_LEN {
            bail!("buffer too small for ipv4 header");
        }
        let version: u8 = 4;
        let ihl: u8 = 5;
        out[0] = (version << 4) | ihl;
        out[1] = dscp_ecn;

        let total_len = (Self::MIN_LEN + payload_len) as u16;
        out[2..4].copy_from_slice(&total_len.to_be_bytes());
        out[4..6].copy_from_slice(&self.identification.to_be_bytes());
        out[6..8].copy_from_slice(&self.flags_fragment.to_be_bytes());
        out[8] = self.ttl;
        out[9] = self.protocol;
        out[10] = 0;
        out[11] = 0;
        out[12..16].copy_from_slice(&self.src.0);
        out[16..20].copy_from_slice(&self.dst.0);

        let csum = ipv4_header_checksum(&out[..Self::MIN_LEN]);
        out[10..12].copy_from_slice(&csum.to_be_bytes());
        Ok(())
    }
}

#[allow(dead_code)]
pub fn ipv4_header_checksum(hdr: &[u8]) -> u16 {
    // hdr length must be a multiple of 2
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < hdr.len() {
        // checksum field must be zero when computing
        if i == 10 {
            i += 2;
            continue;
        }
        let w = u16::from_be_bytes([hdr[i], hdr[i + 1]]) as u32;
        sum = sum.wrapping_add(w);
        i += 2;
    }
    // fold
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
