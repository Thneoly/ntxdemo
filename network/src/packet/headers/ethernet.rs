use anyhow::bail;

#[allow(dead_code)]
pub const ETH_TYPE_IPV4: u16 = 0x0800;
#[allow(dead_code)]
pub const ETH_TYPE_ARP: u16 = 0x0806;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MacAddr(pub [u8; 6]);

#[allow(dead_code)]
impl MacAddr {
    pub const BROADCAST: MacAddr = MacAddr([0xff; 6]);

    pub fn is_broadcast(&self) -> bool {
        self.0 == [0xff; 6]
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct EthernetHeader {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ethertype: u16,
}

#[allow(dead_code)]
impl EthernetHeader {
    pub const LEN: usize = 14;

    /// Decode an Ethernet header from the beginning of `frame`.
    ///
    /// Returns `(header, payload_slice)`.
    pub fn decode(frame: &[u8]) -> anyhow::Result<(Self, &[u8])> {
        if frame.len() < Self::LEN {
            bail!("frame too short for ethernet: {}", frame.len());
        }
        let dst = MacAddr(frame[0..6].try_into().unwrap());
        let src = MacAddr(frame[6..12].try_into().unwrap());
        let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
        Ok((
            EthernetHeader {
                dst,
                src,
                ethertype,
            },
            &frame[Self::LEN..],
        ))
    }

    /// Encode this Ethernet header into `out`.
    pub fn encode(&self, out: &mut [u8]) -> anyhow::Result<()> {
        if out.len() < Self::LEN {
            bail!("buffer too small for ethernet header");
        }
        out[0..6].copy_from_slice(&self.dst.0);
        out[6..12].copy_from_slice(&self.src.0);
        out[12..14].copy_from_slice(&self.ethertype.to_be_bytes());
        Ok(())
    }
}
