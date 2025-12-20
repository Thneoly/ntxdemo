use anyhow::{Context, Result};

use crate::{Ipv4Addr, MacAddr};

pub(crate) fn parse_ipv4(s: &str) -> Result<Ipv4Addr> {
    let parts: Vec<&str> = s.split('.').collect();
    anyhow::ensure!(parts.len() == 4, "ipv4 must have 4 octets");
    let mut octets = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        octets[i] = p
            .parse::<u8>()
            .with_context(|| format!("invalid ipv4 octet: {p}"))?;
    }
    Ok(Ipv4Addr(octets))
}

pub(crate) fn parse_mac(s: &str) -> Result<MacAddr> {
    let parts: Vec<&str> = s.split(':').collect();
    anyhow::ensure!(parts.len() == 6, "mac must be 6 bytes");
    let mut octets = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        octets[i] = u8::from_str_radix(p, 16).with_context(|| format!("invalid mac byte: {p}"))?;
    }
    Ok(MacAddr(octets))
}

pub(crate) fn parse_cidr_v4(cidr: &str) -> Result<(u32, u8)> {
    let (ip_s, prefix_s) = cidr
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("cidr must be like a.b.c.d/p"))?;
    let ip = parse_ipv4(ip_s)?;
    let prefix: u8 = prefix_s.parse::<u8>().with_context(|| "invalid prefix")?;
    anyhow::ensure!(prefix <= 32, "prefix must be <= 32");
    Ok((u32::from_be_bytes(ip.octets()), prefix))
}

/// Return the inclusive host range [start, end] in network-order u32.
///
/// Rules:
/// - /32 => single address
/// - /31 => two addresses (RFC 3021) (no network/broadcast distinction)
/// - <= /30 => exclude network and broadcast (typical host pool semantics)
pub(crate) fn cidr_host_range(net_be: u32, prefix: u8) -> (u32, u32) {
    if prefix == 32 {
        return (net_be, net_be);
    }

    let host_bits = 32 - prefix as u32;
    let mask = if prefix == 0 {
        0u32
    } else {
        (!0u32) << host_bits
    };
    let net = net_be & mask;
    let broadcast = net | (!mask);

    if prefix == 31 {
        return (net, broadcast);
    }

    (net.wrapping_add(1), broadcast.wrapping_sub(1))
}

pub(crate) fn mac_to_u64(mac: MacAddr) -> u64 {
    let b = mac.0;
    ((b[0] as u64) << 40)
        | ((b[1] as u64) << 32)
        | ((b[2] as u64) << 24)
        | ((b[3] as u64) << 16)
        | ((b[4] as u64) << 8)
        | (b[5] as u64)
}

pub(crate) fn u64_to_mac(v: u64) -> MacAddr {
    MacAddr([
        ((v >> 40) & 0xff) as u8,
        ((v >> 32) & 0xff) as u8,
        ((v >> 24) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        (v & 0xff) as u8,
    ])
}
