use anyhow::{Context, Result};
use ntx::network::stack::{Action, Pipeline};
use ntx::network::traffic::udp_echo::UdpEchoHandler;
use ntx::network::{MacAddr, Nic};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    AfPacket,
    TpacketV3,
}

#[derive(Debug, Clone)]
struct Opt {
    /// Network interface to bind AF_PACKET to.
    iface: String,
    /// NIC backend.
    backend: Backend,
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
        backend: Backend::AfPacket,
        port: 10001,
        snaplen: 2048,
        verbose: false,
    };

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                eprintln!(
                    "Usage: userspace-udp-echo [--iface IFACE] [--backend afpacket|tpacketv3] [--port PORT] [--snaplen N] [--verbose]\n\nDefault: --iface eno1 --backend afpacket --port 10001 --snaplen 2048"
                );
                std::process::exit(0);
            }
            "--iface" => {
                if let Some(v) = it.next() {
                    opt.iface = v;
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

    let mut nic: Box<dyn Nic> = match opt.backend {
        Backend::AfPacket => {
            Box::new(ntx::network::AfPacketNic::open(&opt.iface).context("open afpacket nic")?)
        }
        Backend::TpacketV3 => {
            // Defaults tuned for veth/local tests; adjust as needed.
            // block_size must be power-of-two.
            Box::new(
                ntx::network::TpacketV3Nic::open(&opt.iface, 1 << 20, 64, 2048, 10)
                    .context("open tpacketv3 nic")?,
            )
        }
    };

    let iface_mac = nic
        .iface_mac()
        .context("failed to query iface mac (SIOCGIFHWADDR); are you running as root?")?;

    eprintln!(
        "userspace-udp-echo: backend={:?} iface={} ifindex={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} port={} (sudo required)",
        opt.backend,
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

    // Pluggable pipeline: decode once, run handlers.
    let mut pipeline = Pipeline::new();
    pipeline.add_handler(UdpEchoHandler {
        listen_port: opt.port,
        iface_mac: MacAddr(iface_mac),
        verbose: opt.verbose,
    });

    let mut rx_cnt: u64 = 0;
    let mut echo_cnt: u64 = 0;
    let mut last_report = std::time::Instant::now();
    let mut last_rx_cnt: u64 = 0;
    let mut last_echo_cnt: u64 = 0;
    let report_iv = std::time::Duration::from_secs(1);

    loop {
        let n = match nic.recv(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("recv error: {e:#}");
                continue;
            }
        };
        rx_cnt = rx_cnt.wrapping_add(1);
        let action = match pipeline.process(&buf[..n]) {
            Ok(a) => a,
            Err(_) => continue,
        };

        match action {
            Action::Pass => {}
            Action::Reply(reply) => {
                echo_cnt = echo_cnt.wrapping_add(1);
                if let Err(e) = nic.send(&reply.bytes) {
                    eprintln!("send error: {e:#}");
                }
            }
        }

        // Periodic stats. Print after processing so echoed reflects replies actually sent.
        if last_report.elapsed() >= report_iv {
            let rx_delta = rx_cnt.wrapping_sub(last_rx_cnt);
            let echo_delta = echo_cnt.wrapping_sub(last_echo_cnt);
            eprintln!(
                "stats: rx={} echoed={} (+rx={} +echoed={} in last {:?})",
                rx_cnt, echo_cnt, rx_delta, echo_delta, report_iv
            );
            last_rx_cnt = rx_cnt;
            last_echo_cnt = echo_cnt;
            last_report = std::time::Instant::now();
        }
    }
}
