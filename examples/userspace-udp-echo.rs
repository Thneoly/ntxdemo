use anyhow::{Context, Result};
use ntx::network::{AfPacketNic, ETH_TYPE_IPV4, EthernetHeader, Ipv4Header, MacAddr, UdpHeader};

const IP_PROTO_UDP: u8 = 17;

#[derive(Debug, Clone)]
struct Opt {
    /// Network interface to bind AF_PACKET to.
    iface: String,
    /// UDP port to echo.
    port: u16,
    /// Maximum frame size to read per recv().
    snaplen: usize,
    /// Print per-packet logs when a packet is echoed.
    verbose: bool,
}

fn parse_args() -> Opt {
    let mut opt = Opt {
        iface: "eno1".to_string(),
        port: 10001,
        snaplen: 2048,
        verbose: false,
    };

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                eprintln!(
                    "Usage: userspace-udp-echo [--iface IFACE] [--port PORT] [--snaplen N] [--verbose]\n\nDefault: --iface eno1 --port 10001 --snaplen 2048"
                );
                std::process::exit(0);
            }
            "--iface" => {
                if let Some(v) = it.next() {
                    opt.iface = v;
                }
            }
            "--port" => {
                if let Some(v) = it.next() {
                    if let Ok(p) = v.parse::<u16>() {
                        opt.port = p;
                    }
                }
            }
            "--snaplen" => {
                if let Some(v) = it.next() {
                    if let Ok(n) = v.parse::<usize>() {
                        opt.snaplen = n;
                    }
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

fn main() -> Result<()> {
    let opt = parse_args();

    let nic = AfPacketNic::open(&opt.iface).context("open afpacket nic")?;

    let iface_mac = nic
        .iface_mac()
        .context("failed to query iface mac (SIOCGIFHWADDR); are you running as root?")?;

    eprintln!(
        "userspace-udp-echo: iface={} ifindex={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} port={} (sudo required)",
        nic.ifname(),
        nic.ifindex(),
        iface_mac[0],
        iface_mac[1],
        iface_mac[2],
        iface_mac[3],
        iface_mac[4],
        iface_mac[5],
        opt.port
    );

    // We don't own IP; we act as a reflector.
    // Only echo frames addressed to our MAC OR broadcast.

    let mut buf = vec![0u8; opt.snaplen];
    let mut out = vec![0u8; opt.snaplen];

    let mut rx_cnt: u64 = 0;
    let mut echo_cnt: u64 = 0;
    let mut last_report = std::time::Instant::now();

    loop {
        let n = match nic.recv(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("recv error: {e:#}");
                continue;
            }
        };
        rx_cnt = rx_cnt.wrapping_add(1);

        if opt.verbose && last_report.elapsed() >= std::time::Duration::from_secs(1) {
            eprintln!("stats: rx={} echoed={} (last 1s)", rx_cnt, echo_cnt);
            last_report = std::time::Instant::now();
        }
        if n < EthernetHeader::LEN {
            continue;
        }
        let frame = &buf[..n];

        let (eth, l3) = match EthernetHeader::parse(frame) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Filter: IPv4 only.
        if eth.ethertype != ETH_TYPE_IPV4 {
            continue;
        }

        // Filter: dst == iface mac OR broadcast.
        if !eth.dst.is_broadcast() && eth.dst.0 != iface_mac {
            continue;
        }

        let (ip, l4) = match Ipv4Header::parse(l3) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if ip.protocol != IP_PROTO_UDP {
            continue;
        }

        let (udp, payload) = match UdpHeader::parse(l4) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if udp.dst_port != opt.port {
            continue;
        }

        echo_cnt = echo_cnt.wrapping_add(1);

        if opt.verbose {
            eprintln!(
                "echo hit: eth {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}; ip {}.{}.{}.{}:{} -> {}.{}.{}.{}:{}; payload_len={}",
                eth.src.0[0],
                eth.src.0[1],
                eth.src.0[2],
                eth.src.0[3],
                eth.src.0[4],
                eth.src.0[5],
                eth.dst.0[0],
                eth.dst.0[1],
                eth.dst.0[2],
                eth.dst.0[3],
                eth.dst.0[4],
                eth.dst.0[5],
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
                payload.len()
            );
        }

        // Build reply: swap mac/ip/port.
        let reply_eth = EthernetHeader {
            dst: eth.src,
            src: MacAddr(iface_mac),
            ethertype: ETH_TYPE_IPV4,
        };

        let reply_ip = Ipv4Header {
            src: ip.dst,
            dst: ip.src,
            protocol: IP_PROTO_UDP,
            ttl: 64,
            identification: 0,
            flags_fragment: 0,
        };

        let reply_udp = UdpHeader {
            src_port: udp.dst_port,
            dst_port: udp.src_port,
        };

        let eth_len = EthernetHeader::LEN;
        let ip_len = Ipv4Header::MIN_LEN;
        let udp_len = UdpHeader::LEN;

        let needed = eth_len + ip_len + udp_len + payload.len();
        if needed > out.len() {
            continue;
        }

        // Ethernet
        reply_eth.write(&mut out[..eth_len])?;

        // IPv4
        reply_ip.write(
            &mut out[eth_len..eth_len + ip_len],
            udp_len + payload.len(),
            0,
        )?;

        // UDP
        let udp_off = eth_len + ip_len;
        reply_udp.write(
            &mut out[udp_off..udp_off + udp_len + payload.len()],
            payload,
            reply_ip.src,
            reply_ip.dst,
        )?;

        // Send
        if let Err(e) = nic.send(&out[..needed]) {
            eprintln!("send error: {e:#}");
        }
    }
}
