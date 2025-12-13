use anyhow::{Context, Result};
use ntx::network::stack::{Action, PacketContext, Pipeline, UdpEchoHandler};
use ntx::network::{AfPacketNic, MacAddr};

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

    // Pluggable pipeline: decode once, run handlers.
    let mut ctx = PacketContext::with_capacity(opt.snaplen);
    let mut pipeline = Pipeline::new();
    pipeline.add_handler(UdpEchoHandler {
        listen_port: opt.port,
        iface_mac: MacAddr(iface_mac),
        verbose: opt.verbose,
    });

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
        ctx.set_frame(&buf[..n]);
        let action = match pipeline.process(&ctx) {
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
    }
}
