use crate::{EthernetHeader, Ipv4Header, TcpHeader, UdpHeader};

/// Parsed view of a received frame.
///
/// The slices reference the original receive buffer.
#[derive(Debug, Clone)]
pub struct DecodedPacket<'a> {
    pub eth: EthernetHeader,
    pub ip: Option<Ipv4Header>,
    pub udp: Option<UdpHeader>,
    pub tcp: Option<TcpHeader>,
    pub payload: &'a [u8],
}

/// A packet context that owns the raw frame bytes and caches decode results.
///
/// This makes it easy to plug multiple handlers without re-parsing.
#[derive(Debug)]
pub struct PacketContext {
    /// Raw frame bytes.
    pub frame: Vec<u8>,
}

impl PacketContext {
    pub fn new() -> Self {
        Self { frame: Vec::new() }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            frame: Vec::with_capacity(cap),
        }
    }

    /// Replace the frame bytes with the provided slice.
    pub fn set_frame(&mut self, data: &[u8]) {
        self.frame.clear();
        self.frame.extend_from_slice(data);
    }

    pub fn len(&self) -> usize {
        self.frame.len()
    }

    /// Decode Ethernet → (IPv4) → (UDP).
    ///
    /// On success returns a view referencing `self.frame`.
    pub fn decode(&self) -> anyhow::Result<DecodedPacket<'_>> {
        let (eth, l3) = EthernetHeader::parse(&self.frame)?;

        // Default: only Ethernet.
        let mut decoded = DecodedPacket {
            eth,
            ip: None,
            udp: None,
            tcp: None,
            payload: l3,
        };

        // Try IPv4.
        if eth.ethertype == crate::ETH_TYPE_IPV4 {
            let (ip, l4) = Ipv4Header::parse(l3)?;
            decoded.ip = Some(ip);
            decoded.payload = l4;

            // Try UDP.
            if ip.protocol == 17 {
                let (udp, pl) = UdpHeader::parse(l4)?;
                decoded.udp = Some(udp);
                decoded.payload = pl;
            }

            // Try TCP.
            if ip.protocol == 6 {
                let (tcp, pl) = TcpHeader::parse(l4)?;
                decoded.tcp = Some(tcp);
                decoded.payload = pl;
            }
        }

        Ok(decoded)
    }
}
