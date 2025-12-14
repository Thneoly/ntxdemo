use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::Ipv4Addr;

/// MVP-4 scenario file.
///
/// YAML example:
///
/// ```yaml
/// version: 1
/// iface: eno1
/// dst_ip_file: dst_ips.txt
/// src_ip: 192.168.1.10
/// src_port: 40000
/// dst_port: 10001
/// payload: "ntx-traffic"
/// pps: 1000
/// count: 10000
///
/// arp:
///   enabled: true
///   timeout_ms: 800
///   ttl_s: 60
///
/// rr:
///   enabled: true
///   timeout_ms: 500
///   poll_budget: 256
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct Scenario {
    #[serde(default = "default_version")]
    pub version: u32,

    pub iface: Option<String>,
    pub dst_ip_file: Option<String>,

    /// Destination IPv4 list.
    ///
    /// Each entry can be:
    /// - Single IPv4: "192.168.1.10"
    /// - CIDR: "192.168.1.0/24"
    /// - Range (inclusive): "192.168.1.10-192.168.1.20"
    ///
    /// If both `dst_ips` and `dst_ip_file` are provided, `dst_ips` takes precedence.
    pub dst_ips: Option<Vec<String>>,

    pub src_ip: Option<String>,
    pub src_port: Option<u16>,
    pub dst_port: Option<u16>,

    pub payload: Option<String>,

    pub pps: Option<u64>,
    pub count: Option<u64>,

    pub dst_mac: Option<String>,

    #[serde(default)]
    pub arp: ArpScenario,

    #[serde(default)]
    pub rr: RrScenario,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArpScenario {
    pub enabled: Option<bool>,
    pub timeout_ms: Option<u64>,
    pub ttl_s: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RrScenario {
    pub enabled: Option<bool>,
    pub timeout_ms: Option<u64>,
    pub poll_budget: Option<u32>,
}

pub fn load_scenario(path: impl AsRef<Path>) -> anyhow::Result<Scenario> {
    let path = path.as_ref();
    let s = std::fs::read_to_string(path)
        .with_context(|| format!("read scenario file: {}", path.display()))?;
    let sc: Scenario = serde_yaml::from_str(&s)
        .with_context(|| format!("parse yaml scenario: {}", path.display()))?;
    anyhow::ensure!(
        sc.version == 1,
        "unsupported scenario version: {}",
        sc.version
    );
    Ok(sc)
}

/// Expand scenario `dst_ips` entries into concrete IPv4 addresses.
///
/// This is intended for config-time expansion (not per-packet).
///
/// Guards:
/// - CIDR must be /0..=32
/// - Refuses to expand above `max_ips` to avoid accidental huge allocations
pub fn expand_dst_ips(entries: &[String], max_ips: usize) -> anyhow::Result<Vec<Ipv4Addr>> {
    let mut out = Vec::new();
    for e in entries {
        let e = e.trim();
        if e.is_empty() {
            continue;
        }
        let expanded = expand_one(e)?;
        anyhow::ensure!(
            out.len() + expanded.len() <= max_ips,
            "dst_ips expands to too many IPs (>{max_ips}); refusing"
        );
        out.extend(expanded);
    }
    anyhow::ensure!(!out.is_empty(), "dst_ips expands to empty list");
    Ok(out)
}

fn expand_one(s: &str) -> anyhow::Result<Vec<Ipv4Addr>> {
    // Range: a.b.c.d-e.f.g.h
    if let Some((a, b)) = s.split_once('-') {
        let start = parse_ipv4(a).with_context(|| format!("invalid ipv4 range start: {a}"))?;
        let end = parse_ipv4(b).with_context(|| format!("invalid ipv4 range end: {b}"))?;
        return expand_range(start, end);
    }

    // CIDR: a.b.c.d/prefix
    if let Some((ip_s, p_s)) = s.split_once('/') {
        let ip = parse_ipv4(ip_s).with_context(|| format!("invalid cidr ip: {ip_s}"))?;
        let prefix = p_s
            .trim()
            .parse::<u8>()
            .with_context(|| format!("invalid cidr prefix: {p_s}"))?;
        anyhow::ensure!(prefix <= 32, "cidr prefix must be <= 32, got {prefix}");
        return expand_cidr(ip, prefix);
    }

    // Single IPv4
    let ip = parse_ipv4(s).with_context(|| format!("invalid ipv4: {s}"))?;
    Ok(vec![ip])
}

fn parse_ipv4(s: &str) -> Option<Ipv4Addr> {
    let parts: Vec<_> = s.trim().split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut o = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        o[i] = p.parse::<u8>().ok()?;
    }
    Some(Ipv4Addr(o))
}

fn ipv4_to_u32(ip: Ipv4Addr) -> u32 {
    let o = ip.0;
    ((o[0] as u32) << 24) | ((o[1] as u32) << 16) | ((o[2] as u32) << 8) | (o[3] as u32)
}

fn u32_to_ipv4(v: u32) -> Ipv4Addr {
    Ipv4Addr([
        ((v >> 24) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        (v & 0xff) as u8,
    ])
}

fn expand_range(start: Ipv4Addr, end: Ipv4Addr) -> anyhow::Result<Vec<Ipv4Addr>> {
    let a = ipv4_to_u32(start);
    let b = ipv4_to_u32(end);
    anyhow::ensure!(a <= b, "ipv4 range start must be <= end");
    let len = (b - a) as usize + 1;
    let mut out = Vec::with_capacity(len);
    for v in a..=b {
        out.push(u32_to_ipv4(v));
    }
    Ok(out)
}

fn expand_cidr(ip: Ipv4Addr, prefix: u8) -> anyhow::Result<Vec<Ipv4Addr>> {
    let ip_u = ipv4_to_u32(ip);
    let mask = if prefix == 0 {
        0u32
    } else {
        (!0u32) << (32 - prefix)
    };
    let network = ip_u & mask;
    let size: u64 = 1u64 << (32 - prefix);
    anyhow::ensure!(size <= (usize::MAX as u64), "cidr too large to expand");
    let mut out = Vec::with_capacity(size as usize);
    for i in 0..size {
        out.push(u32_to_ipv4(network + (i as u32)));
    }
    Ok(out)
}
