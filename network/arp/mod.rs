use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

use crate::network::{ETH_TYPE_ARP, EthernetHeader, Ipv4Addr, MacAddr};

/// Ethernet broadcast MAC.
#[allow(dead_code)]
pub const MAC_BROADCAST: MacAddr = MacAddr([0xff; 6]);

/// ARP opcodes.
#[allow(dead_code)]
pub const ARP_OP_REQUEST: u16 = 1;
#[allow(dead_code)]
pub const ARP_OP_REPLY: u16 = 2;

/// A minimal IPv4 over Ethernet ARP packet.
///
/// Layout (RFC826 + Ethernet/IPv4 conventions):
/// - htype: 1 (Ethernet)
/// - ptype: 0x0800 (IPv4)
/// - hlen: 6
/// - plen: 4
/// - oper: 1 or 2
/// - sha/spa/tha/tpa
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArpPacket {
    pub oper: u16,
    pub sha: MacAddr,
    pub spa: Ipv4Addr,
    pub tha: MacAddr,
    pub tpa: Ipv4Addr,
}

#[allow(dead_code)]
impl ArpPacket {
    pub const LEN: usize = 28;

    pub fn parse(payload: &[u8]) -> anyhow::Result<Self> {
        if payload.len() < Self::LEN {
            bail!("arp payload too short: {}", payload.len());
        }
        let htype = u16::from_be_bytes([payload[0], payload[1]]);
        let ptype = u16::from_be_bytes([payload[2], payload[3]]);
        let hlen = payload[4];
        let plen = payload[5];
        if htype != 1 || ptype != 0x0800 || hlen != 6 || plen != 4 {
            bail!(
                "unsupported arp: htype={} ptype=0x{:04x} hlen={} plen={}",
                htype,
                ptype,
                hlen,
                plen
            );
        }

        let oper = u16::from_be_bytes([payload[6], payload[7]]);
        let sha = MacAddr(payload[8..14].try_into().unwrap());
        let spa = Ipv4Addr(payload[14..18].try_into().unwrap());
        let tha = MacAddr(payload[18..24].try_into().unwrap());
        let tpa = Ipv4Addr(payload[24..28].try_into().unwrap());

        Ok(Self {
            oper,
            sha,
            spa,
            tha,
            tpa,
        })
    }

    pub fn write(&self, out: &mut [u8]) -> anyhow::Result<()> {
        if out.len() < Self::LEN {
            bail!("buffer too small for arp: {}", out.len());
        }
        out[0..2].copy_from_slice(&1u16.to_be_bytes());
        out[2..4].copy_from_slice(&0x0800u16.to_be_bytes());
        out[4] = 6;
        out[5] = 4;
        out[6..8].copy_from_slice(&self.oper.to_be_bytes());
        out[8..14].copy_from_slice(&self.sha.0);
        out[14..18].copy_from_slice(&self.spa.0);
        out[18..24].copy_from_slice(&self.tha.0);
        out[24..28].copy_from_slice(&self.tpa.0);
        Ok(())
    }
}

/// Build an Ethernet + ARP request frame asking for `target_ip`.
#[allow(dead_code)]
pub fn build_arp_request_frame(
    src_mac: MacAddr,
    src_ip: Ipv4Addr,
    target_ip: Ipv4Addr,
) -> anyhow::Result<Vec<u8>> {
    let eth = EthernetHeader {
        dst: MAC_BROADCAST,
        src: src_mac,
        ethertype: ETH_TYPE_ARP,
    };
    let arp = ArpPacket {
        oper: ARP_OP_REQUEST,
        sha: src_mac,
        spa: src_ip,
        tha: MacAddr([0, 0, 0, 0, 0, 0]),
        tpa: target_ip,
    };

    let mut bytes = vec![0u8; EthernetHeader::LEN + ArpPacket::LEN];
    eth.write(&mut bytes[..EthernetHeader::LEN])?;
    arp.write(&mut bytes[EthernetHeader::LEN..])?;
    Ok(bytes)
}

/// Best-effort parse for Ethernet + ARP reply.
///
/// Returns `Some((sender_ip, sender_mac))` if this is an ARP reply.
#[allow(dead_code)]
pub fn parse_arp_reply(frame: &[u8]) -> anyhow::Result<Option<(Ipv4Addr, MacAddr)>> {
    let (eth, payload) = EthernetHeader::parse(frame).context("parse ethernet")?;
    if eth.ethertype != ETH_TYPE_ARP {
        return Ok(None);
    }
    let arp = ArpPacket::parse(payload).context("parse arp")?;
    if arp.oper != ARP_OP_REPLY {
        return Ok(None);
    }
    Ok(Some((arp.spa, arp.sha)))
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
struct CacheEntry {
    mac: MacAddr,
    expires_at: Instant,
}

/// A very small ARP cache for IPv4 -> MAC.
#[allow(dead_code)]
pub struct ArpCache {
    ttl: Duration,
    map: HashMap<[u8; 4], CacheEntry>,
}

#[allow(dead_code)]
impl ArpCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            map: HashMap::new(),
        }
    }

    pub fn insert(&mut self, ip: Ipv4Addr, mac: MacAddr) {
        self.map.insert(
            ip.0,
            CacheEntry {
                mac,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }

    pub fn get(&mut self, ip: Ipv4Addr) -> Option<MacAddr> {
        self.reap_expired();
        self.map.get(&ip.0).and_then(|e| {
            if Instant::now() <= e.expires_at {
                Some(e.mac)
            } else {
                None
            }
        })
    }

    pub fn reap_expired(&mut self) {
        let now = Instant::now();
        self.map.retain(|_, v| now <= v.expires_at);
    }
}
