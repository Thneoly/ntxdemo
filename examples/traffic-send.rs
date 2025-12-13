use anyhow::{Context, Result};
use ntx::network::arp::{ArpCache, MAC_BROADCAST, build_arp_request_frame, parse_arp_reply};
use ntx::network::traffic::matcher::{FlowKey, Matcher};
use ntx::network::traffic::scenario::{expand_dst_ips, load_scenario};
use ntx::network::traffic::token::{TOKEN_LEN, Token, decode_token, encode_token};
use ntx::network::{ETH_TYPE_IPV4, EthernetHeader, Ipv4Addr, Ipv4Header, MacAddr, Nic, UdpHeader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    AfPacket,
    TpacketV3,
}

#[derive(Debug, Clone)]
struct Opt {
    /// Optional scenario YAML file.
    scenario: Option<String>,
    /// NIC backend.
    backend: Backend,
    iface: String,
    /// Destination IPv4 list entries from CLI.
    ///
    /// Same syntax as scenario `dst_ips`: single IP / CIDR / range.
    /// Can be repeated, and each value may be comma-separated.
    dst_ips: Vec<String>,
    /// Destination IPv4 list file (one IP per line).
    dst_ip_file: String,
    /// Optional source IPv4.
    src_ip: Option<Ipv4Addr>,
    /// Source UDP port.
    src_port: u16,
    /// Destination UDP port.
    dst_port: u16,
    /// Payload bytes (utf-8 string) to send.
    payload: Vec<u8>,
    /// Packets per second (0 = as fast as possible).
    pps: u64,
    /// Number of packets to send (0 = infinite).
    count: u64,
    /// Optional destination MAC (aa:bb:cc:dd:ee:ff).
    dst_mac: Option<MacAddr>,

    /// Enable ARP resolve for dst_ip -> dst_mac when --dst-mac is not provided.
    arp: bool,
    /// ARP reply wait timeout per (re)query.
    arp_timeout_ms: u64,
    /// ARP cache TTL seconds.
    arp_ttl_s: u64,

    /// Enable request-reply matching: embed token in payload and wait for replies.
    rr: bool,
    /// Timeout for an outstanding request (milliseconds).
    rr_timeout_ms: u64,
    /// Maximum receive polling budget per loop iteration.
    rr_poll_budget: u32,
    verbose: bool,
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

fn parse_mac(s: &str) -> Option<MacAddr> {
    let parts: Vec<_> = s.trim().split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut o = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        o[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(MacAddr(o))
}

fn parse_args() -> Opt {
    let mut opt = Opt {
        scenario: None,
        backend: Backend::AfPacket,
        iface: "eno1".to_string(),
        dst_ips: Vec::new(),
        dst_ip_file: "".to_string(),
        src_ip: None,
        src_port: 40000,
        dst_port: 10001,
        payload: b"ntx-traffic".to_vec(),
        pps: 0,
        count: 0,
        dst_mac: None,
        arp: false,
        arp_timeout_ms: 800,
        arp_ttl_s: 60,
        rr: false,
        rr_timeout_ms: 500,
        rr_poll_budget: 256,
        verbose: false,
    };

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                eprintln!(
                    "Usage: traffic-send [--scenario FILE] --iface IFACE [--backend afpacket|tpacketv3] (--dst-ips SPEC | --dst-ip-file FILE) [--src-ip A.B.C.D] [--src-port P] [--dst-port P]\n\
                     	[--payload STRING] [--pps N] [--count N] [--dst-mac aa:bb:cc:dd:ee:ff] [--arp]\n\
	[--arp-timeout-ms MS] [--arp-ttl-s S]\n\
	[--rr] [--rr-timeout-ms MS] [--rr-poll-budget N] [--verbose]\n\n\
                     Notes:\n\
                     - This is a userspace L2 UDP traffic generator using AF_PACKET.\n\
                     - It supports ARP resolution (--arp) and request-reply matching (--rr) for RTT/timeouts.\n\
                     - MVP-4: --scenario loads YAML and provides defaults; CLI flags override scenario.\n\
                     - --dst-ips supports: single IP (10.0.0.1), CIDR (10.0.0.0/24), range (10.0.0.10-10.0.0.20).\n\
                     - With --arp enabled and without --dst-mac, we send ARP requests and learn dst MACs.\n\
                     - If neither --dst-mac nor --arp is provided, dst MAC defaults to broadcast (ff:ff:ff:ff:ff:ff).\n\
                     - --backend selects RX backend: afpacket (copy) or tpacketv3 (PACKET_RX_RING).\n\
                     - With --rr enabled, we prepend a 12-byte token (\"NTX1\" + u64 seq) into UDP payload,\n\
                                             then match replies by (dst_ip,dst_port,src_port,token) and report RTT/timeouts.\n\n\
                                         Examples:\n\
                                         - Send to CIDR targets (requires root/cap_net_raw):\n\
                                             sudo traffic-send --iface eno1 --dst-ips 10.0.0.0/24 --pps 1000 --count 10000\n\
                                         - Mix single/CIDR/range, repeat --dst-ips, and allow comma-separated specs:\n\
                                             sudo traffic-send --iface eno1 --dst-ips 10.0.0.1,10.0.0.2 --dst-ips 10.0.1.0/30 --dst-ips 10.0.2.10-10.0.2.12\n\
                                         - Scenario (YAML) + override a field from CLI:\n\
                                             sudo traffic-send --scenario scenario.yaml --iface eno1 --dst-ips 10.0.0.0/24"
                );
                std::process::exit(0);
            }
            "--scenario" => {
                if let Some(v) = it.next() {
                    opt.scenario = Some(v);
                }
            }
            "--backend" => {
                if let Some(v) = it.next() {
                    match v.as_str() {
                        "afpacket" => opt.backend = Backend::AfPacket,
                        "tpacketv3" | "tpacket_v3" | "tpv3" => opt.backend = Backend::TpacketV3,
                        _ => {
                            eprintln!("invalid --backend: {v} (expected: afpacket|tpacketv3)");
                            std::process::exit(2);
                        }
                    }
                }
            }
            "--iface" => {
                if let Some(v) = it.next() {
                    opt.iface = v;
                }
            }
            "--dst-ips" => {
                if let Some(v) = it.next() {
                    for part in v.split(',') {
                        let t = part.trim();
                        if !t.is_empty() {
                            opt.dst_ips.push(t.to_string());
                        }
                    }
                }
            }
            "--dst-ip-file" => {
                if let Some(v) = it.next() {
                    opt.dst_ip_file = v;
                }
            }
            "--src-ip" => {
                if let Some(v) = it.next() {
                    opt.src_ip = parse_ipv4(&v);
                }
            }
            "--src-port" => {
                if let Some(v) = it.next() {
                    if let Ok(p) = v.parse::<u16>() {
                        opt.src_port = p;
                    }
                }
            }
            "--dst-port" => {
                if let Some(v) = it.next() {
                    if let Ok(p) = v.parse::<u16>() {
                        opt.dst_port = p;
                    }
                }
            }
            "--payload" => {
                if let Some(v) = it.next() {
                    opt.payload = v.into_bytes();
                }
            }
            "--pps" => {
                if let Some(v) = it.next() {
                    opt.pps = v.parse::<u64>().unwrap_or(0);
                }
            }
            "--count" => {
                if let Some(v) = it.next() {
                    opt.count = v.parse::<u64>().unwrap_or(0);
                }
            }
            "--dst-mac" => {
                if let Some(v) = it.next() {
                    opt.dst_mac = parse_mac(&v);
                }
            }
            "--arp" => {
                opt.arp = true;
            }
            "--arp-timeout-ms" => {
                if let Some(v) = it.next() {
                    opt.arp_timeout_ms = v.parse::<u64>().unwrap_or(opt.arp_timeout_ms);
                }
            }
            "--arp-ttl-s" => {
                if let Some(v) = it.next() {
                    opt.arp_ttl_s = v.parse::<u64>().unwrap_or(opt.arp_ttl_s);
                }
            }
            "--rr" => {
                opt.rr = true;
            }
            "--rr-timeout-ms" => {
                if let Some(v) = it.next() {
                    opt.rr_timeout_ms = v.parse::<u64>().unwrap_or(opt.rr_timeout_ms);
                }
            }
            "--rr-poll-budget" => {
                if let Some(v) = it.next() {
                    opt.rr_poll_budget = v.parse::<u32>().unwrap_or(opt.rr_poll_budget);
                }
            }
            "--verbose" => {
                opt.verbose = true;
            }
            _ => {}
        }
    }

    opt
}

fn load_dst_ips(path: &str) -> Result<Vec<Ipv4Addr>> {
    let s = std::fs::read_to_string(path).with_context(|| format!("read dst ip file: {path}"))?;
    let mut ips = Vec::new();
    for (idx, line) in s.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let ip = parse_ipv4(t).with_context(|| format!("invalid ipv4 at line {}: {t}", idx + 1))?;
        ips.push(ip);
    }
    anyhow::ensure!(!ips.is_empty(), "no dst ips loaded from {path}");
    Ok(ips)
}

fn main() -> Result<()> {
    let mut opt = parse_args();

    // Destination IP resolution precedence (highest to lowest):
    // 1) CLI --dst-ips
    // 2) scenario.dst_ips
    // 3) CLI --dst-ip-file
    // 4) scenario.dst_ip_file
    let mut scenario_dst_ips: Option<Vec<Ipv4Addr>> = None;
    let mut dst_ips_cli: Option<Vec<Ipv4Addr>> = None;
    if !opt.dst_ips.is_empty() {
        dst_ips_cli = Some(expand_dst_ips(&opt.dst_ips, 65_536).context("expand cli --dst-ips")?);
    }

    // MVP-4: load scenario defaults. CLI flags override scenario.
    if let Some(path) = opt.scenario.clone() {
        let sc = load_scenario(&path).context("load scenario")?;

        if opt.iface == "eno1" {
            if let Some(v) = sc.iface {
                opt.iface = v;
            }
        }
        // dst ip sources
        if scenario_dst_ips.is_none() {
            if let Some(entries) = sc.dst_ips {
                // Guard against accidental huge expansion.
                scenario_dst_ips =
                    Some(expand_dst_ips(&entries, 65_536).context("expand scenario dst_ips")?);
            }
        }
        if opt.dst_ip_file.is_empty() {
            if let Some(v) = sc.dst_ip_file {
                opt.dst_ip_file = v;
            }
        }
        if opt.src_ip.is_none() {
            if let Some(v) = sc.src_ip {
                opt.src_ip = parse_ipv4(&v);
            }
        }
        if opt.src_port == 40000 {
            if let Some(v) = sc.src_port {
                opt.src_port = v;
            }
        }
        if opt.dst_port == 10001 {
            if let Some(v) = sc.dst_port {
                opt.dst_port = v;
            }
        }
        if opt.payload == b"ntx-traffic".to_vec() {
            if let Some(v) = sc.payload {
                opt.payload = v.into_bytes();
            }
        }
        if opt.pps == 0 {
            if let Some(v) = sc.pps {
                opt.pps = v;
            }
        }
        if opt.count == 0 {
            if let Some(v) = sc.count {
                opt.count = v;
            }
        }
        if opt.dst_mac.is_none() {
            if let Some(v) = sc.dst_mac {
                opt.dst_mac = parse_mac(&v);
            }
        }

        // arp
        if !opt.arp {
            if let Some(v) = sc.arp.enabled {
                opt.arp = v;
            }
        }
        if opt.arp_timeout_ms == 800 {
            if let Some(v) = sc.arp.timeout_ms {
                opt.arp_timeout_ms = v;
            }
        }
        if opt.arp_ttl_s == 60 {
            if let Some(v) = sc.arp.ttl_s {
                opt.arp_ttl_s = v;
            }
        }

        // rr
        if !opt.rr {
            if let Some(v) = sc.rr.enabled {
                opt.rr = v;
            }
        }
        if opt.rr_timeout_ms == 500 {
            if let Some(v) = sc.rr.timeout_ms {
                opt.rr_timeout_ms = v;
            }
        }
        if opt.rr_poll_budget == 256 {
            if let Some(v) = sc.rr.poll_budget {
                opt.rr_poll_budget = v;
            }
        }
    }
    anyhow::ensure!(
        dst_ips_cli.is_some() || scenario_dst_ips.is_some() || !opt.dst_ip_file.is_empty(),
        "either --dst-ips, scenario.dst_ips, or --dst-ip-file is required (see --help)"
    );

    let mut nic: Box<dyn Nic> = match opt.backend {
        Backend::AfPacket => {
            Box::new(ntx::network::AfPacketNic::open(&opt.iface).context("open afpacket nic")?)
        }
        Backend::TpacketV3 => Box::new(
            ntx::network::TpacketV3Nic::open(&opt.iface, 1 << 20, 64, 2048, 10)
                .context("open tpacketv3 nic")?,
        ),
    };

    let iface_mac = nic
        .iface_mac()
        .context("failed to query iface mac (SIOCGIFHWADDR); are you running as root?")?;

    let dst_ips = if let Some(v) = dst_ips_cli {
        v
    } else if let Some(v) = scenario_dst_ips {
        v
    } else {
        load_dst_ips(&opt.dst_ip_file)?
    };

    let mut arp_cache = ArpCache::new(std::time::Duration::from_secs(opt.arp_ttl_s.max(1)));

    let default_dst_mac = opt.dst_mac.unwrap_or(MAC_BROADCAST);

    eprintln!(
        "traffic-send (MVP-3 rr={} arp={}): backend={:?} iface={} ifindex={} src_mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} default_dst_mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} dst_ips={} dst_port={} src_port={} pps={} count={} (sudo required)",
        opt.rr,
        opt.arp,
        opt.backend,
        nic.ifname(),
        nic.ifindex(),
        iface_mac[0],
        iface_mac[1],
        iface_mac[2],
        iface_mac[3],
        iface_mac[4],
        iface_mac[5],
        default_dst_mac.0[0],
        default_dst_mac.0[1],
        default_dst_mac.0[2],
        default_dst_mac.0[3],
        default_dst_mac.0[4],
        default_dst_mac.0[5],
        dst_ips.len(),
        opt.dst_port,
        opt.src_port,
        opt.pps,
        opt.count,
    );

    // Rate control.
    let mut next_deadline = std::time::Instant::now();
    let interval = if opt.pps > 0 {
        Some(std::time::Duration::from_nanos(
            1_000_000_000u64 / opt.pps.max(1),
        ))
    } else {
        None
    };

    let mut sent: u64 = 0;
    let mut last_report = std::time::Instant::now();

    let eth_len = EthernetHeader::LEN;
    let ip_len = Ipv4Header::MIN_LEN;
    let udp_len = UdpHeader::LEN;
    let payload_len = opt.payload.len();

    let frame_len = eth_len + ip_len + udp_len + payload_len;
    let mut frame = vec![0u8; frame_len];

    // Best-effort src_ip selection for MVP-1.
    // If not specified, use 0.0.0.0 (valid for building, but may not get a reply).
    let src_ip = opt.src_ip.unwrap_or(Ipv4Addr([0, 0, 0, 0]));

    let mut matcher = Matcher::new(std::time::Duration::from_millis(opt.rr_timeout_ms.max(1)));
    let mut seq: u64 = 1;

    // Receive buffer for rr/arp.
    let mut rbuf = vec![0u8; 4096];

    loop {
        if let Some(n) = opt.count.checked_sub(sent) {
            if opt.count != 0 && n == 0 {
                break;
            }
        }

        // If rr enabled: poll some receives each loop to catch replies and timeouts.
        if opt.rr {
            for _ in 0..opt.rr_poll_budget {
                match nic.recv_nonblocking(&mut rbuf) {
                    Ok(Some(n)) => {
                        // ARP learning (best effort) - keeps cache warm.
                        if let Ok(Some((ip, mac))) = parse_arp_reply(&rbuf[..n]) {
                            arp_cache.insert(ip, mac);
                            continue;
                        }

                        // Try decode UDP token from IPv4 packets.
                        if let Ok((eth, l3)) = EthernetHeader::parse(&rbuf[..n]) {
                            if eth.ethertype != ETH_TYPE_IPV4 {
                                continue;
                            }
                            if let Ok((ip, l4)) = Ipv4Header::parse(l3) {
                                if ip.protocol != 17 {
                                    continue;
                                }
                                if let Ok((udp, payload)) = UdpHeader::parse(l4) {
                                    // Expect reply: src_port == dst_port and dst_port == src_port.
                                    if udp.dst_port != opt.src_port {
                                        continue;
                                    }
                                    if udp.src_port != opt.dst_port {
                                        continue;
                                    }
                                    if let Ok(tok) = decode_token(payload) {
                                        let key = FlowKey {
                                            // For the request, dst_ip was the target. In reply, ip.src is the target.
                                            dst_ip: ip.src,
                                            dst_port: opt.dst_port,
                                            src_port: opt.src_port,
                                            token: tok,
                                        };
                                        matcher.on_reply(key);
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            matcher.sweep_timeouts();
        }

        if let Some(iv) = interval {
            let now = std::time::Instant::now();
            if now < next_deadline {
                std::thread::sleep(next_deadline - now);
            }
            next_deadline += iv;
        }

        let dst_ip = dst_ips[(sent as usize) % dst_ips.len()];

        // Resolve destination MAC.
        let dst_mac = if let Some(m) = opt.dst_mac {
            m
        } else if opt.arp {
            if let Some(m) = arp_cache.get(dst_ip) {
                m
            } else {
                // Send ARP request and wait for reply.
                // NOTE: In MVP-2 we keep this synchronous; later we can make it async/background.
                let req = build_arp_request_frame(MacAddr(iface_mac), src_ip, dst_ip)
                    .context("build arp request")?;
                let _ = nic.send(&req);

                let deadline = std::time::Instant::now()
                    + std::time::Duration::from_millis(opt.arp_timeout_ms.max(1));
                let mut learned: Option<MacAddr> = None;
                while std::time::Instant::now() < deadline {
                    match nic.recv_nonblocking(&mut rbuf) {
                        Ok(Some(n)) => {
                            if let Ok(Some((ip, mac))) = parse_arp_reply(&rbuf[..n]) {
                                if ip == dst_ip {
                                    arp_cache.insert(ip, mac);
                                    learned = Some(mac);
                                    break;
                                }
                            }
                        }
                        Ok(None) => {
                            // No packet ready right now.
                            // Keep this loop cooperative to avoid pegging a CPU.
                            std::thread::yield_now();
                        }
                        Err(_) => {
                            // ignore transient errors
                        }
                    }
                }

                learned.unwrap_or(MAC_BROADCAST)
            }
        } else {
            MAC_BROADCAST
        };

        // Ethernet
        let eth = EthernetHeader {
            dst: dst_mac,
            src: MacAddr(iface_mac),
            ethertype: ETH_TYPE_IPV4,
        };
        eth.write(&mut frame[..eth_len])?;

        // IPv4
        let ip = Ipv4Header {
            src: src_ip,
            dst: dst_ip,
            protocol: 17,
            ttl: 64,
            identification: 0,
            flags_fragment: 0,
        };
        ip.write(
            &mut frame[eth_len..eth_len + ip_len],
            udp_len + payload_len,
            0,
        )?;

        // UDP
        let udp = UdpHeader {
            src_port: opt.src_port,
            dst_port: opt.dst_port,
        };
        let udp_off = eth_len + ip_len;
        // Build payload (rr mode prepends token).
        let mut payload_buf;
        let payload_ref: &[u8] = if opt.rr {
            let tok = encode_token(seq);
            payload_buf = Vec::with_capacity(TOKEN_LEN + opt.payload.len());
            payload_buf.extend_from_slice(&tok);
            payload_buf.extend_from_slice(&opt.payload);
            &payload_buf
        } else {
            &opt.payload
        };

        // Ensure frame buffer large enough if rr changes payload length.
        let needed_len = eth_len + ip_len + udp_len + payload_ref.len();
        if frame.len() != needed_len {
            frame.resize(needed_len, 0);
        }

        // IPv4 + UDP write must use correct payload length.
        ip.write(
            &mut frame[eth_len..eth_len + ip_len],
            udp_len + payload_ref.len(),
            0,
        )?;

        udp.write(
            &mut frame[udp_off..udp_off + udp_len + payload_ref.len()],
            payload_ref,
            ip.src,
            ip.dst,
        )?;

        if let Err(e) = nic.send(&frame) {
            eprintln!("send error: {e:#}");
            // continue sending
        } else {
            sent = sent.wrapping_add(1);
            if opt.rr {
                let key = FlowKey {
                    dst_ip,
                    dst_port: opt.dst_port,
                    src_port: opt.src_port,
                    token: Token(seq),
                };
                matcher.insert(key);
                seq = seq.wrapping_add(1);
            }
        }

        if last_report.elapsed() >= std::time::Duration::from_secs(1) {
            if opt.rr {
                let avg = matcher.stats.rtt.avg_us().unwrap_or(0);
                eprintln!(
                    "stats: sent={} matched={} timeouts={} outstanding={} rtt_us(min/avg/max)={}/{}/{} (last 1s)",
                    matcher.stats.sent,
                    matcher.stats.matched,
                    matcher.stats.timeouts,
                    matcher.outstanding(),
                    matcher.stats.rtt.min_us,
                    avg,
                    matcher.stats.rtt.max_us,
                );
            } else {
                eprintln!("stats: sent={} (last 1s)", sent);
            }
            last_report = std::time::Instant::now();
        }

        if opt.verbose {
            eprintln!(
                "tx: {}.{}.{}.{}:{} -> {}.{}.{}.{}:{} payload_len={} rr={} dst_mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                ip.src.0[0],
                ip.src.0[1],
                ip.src.0[2],
                ip.src.0[3],
                opt.src_port,
                ip.dst.0[0],
                ip.dst.0[1],
                ip.dst.0[2],
                ip.dst.0[3],
                opt.dst_port,
                payload_ref.len(),
                opt.rr,
                dst_mac.0[0],
                dst_mac.0[1],
                dst_mac.0[2],
                dst_mac.0[3],
                dst_mac.0[4],
                dst_mac.0[5],
            );
        }
    }

    if opt.rr {
        // Print a final summary because short runs (e.g., count=50 at pps=50) can finish
        // before the 1s periodic stats triggers.
        matcher.sweep_timeouts();
        let avg = matcher.stats.rtt.avg_us().unwrap_or(0);
        eprintln!(
            "final: sent={} matched={} timeouts={} outstanding={} rtt_us(min/avg/max)={}/{}/{}",
            matcher.stats.sent,
            matcher.stats.matched,
            matcher.stats.timeouts,
            matcher.outstanding(),
            matcher.stats.rtt.min_us,
            avg,
            matcher.stats.rtt.max_us,
        );
    }

    eprintln!("done: sent={}", sent);
    Ok(())
}
