use anyhow::{Context, Result};

use ntx::network::Nic;
use ntx::network::packet::headers::{Ipv4Addr, MacAddr, parse_arp_reply};
use ntx::network::stack::{LayerId, LayerInstance, LayerRegistry, default_registry, parse_packet};

/// Userspace echo client on top of AF_PACKET.
///
/// - iface: ntx0 (host namespace)
/// - IP:    10.0.0.1
/// - target: 10.0.0.2:7
///
/// Flow:
/// 1) ARP resolve 10.0.0.2 -> dst MAC
/// 2) Send UDP echo request to 10.0.0.2:7
/// 3) Wait for UDP reply and print payload
fn main() -> Result<()> {
    let iface = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ntx0".to_string());

    let client_ip = Ipv4Addr([10, 0, 0, 1]);
    let server_ip = Ipv4Addr([10, 0, 0, 2]);
    let port: u16 = 7;

    let mut nic: Box<dyn Nic> =
        Box::new(ntx::network::AfPacketNic::open(&iface).context("open afpacket nic")?);

    let iface_mac = nic
        .iface_mac()
        .context("failed to query iface mac (SIOCGIFHWADDR); are you root?")?;
    let src_mac = MacAddr(iface_mac);

    eprintln!(
        "ntx-echo-client: iface={} ifindex={} ip={}.{}.{}.{} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} target={}.{}.{}.{}:{} (sudo required)",
        nic.ifname(),
        nic.ifindex(),
        client_ip.0[0],
        client_ip.0[1],
        client_ip.0[2],
        client_ip.0[3],
        src_mac.0[0],
        src_mac.0[1],
        src_mac.0[2],
        src_mac.0[3],
        src_mac.0[4],
        src_mac.0[5],
        server_ip.0[0],
        server_ip.0[1],
        server_ip.0[2],
        server_ip.0[3],
        port
    );

    let reg: LayerRegistry = default_registry();

    // --- 1) ARP resolve ---
    let mut dst_mac: Option<MacAddr> = None;
    let mac_broadcast = MacAddr([0xff; 6]);
    let arp_req_layers = [
        LayerInstance {
            id: LayerId::Ether,
            inner: Box::new(ntx::network::stack::layers::Ether {
                dst: mac_broadcast,
                src: src_mac,
                ethertype: ntx::network::ETH_TYPE_ARP,
            }),
        },
        LayerInstance {
            id: LayerId::Arp,
            inner: Box::new(ntx::network::stack::layers::Arp {
                oper: 1,
                sha: src_mac,
                spa: client_ip,
                tha: MacAddr([0, 0, 0, 0, 0, 0]),
                tpa: server_ip,
            }),
        },
    ];
    let arp_req = ntx::network::stack::build_packet_no_payload(&arp_req_layers, &reg)
        .map_err(anyhow::Error::msg)
        .context("build arp request")?;
    let mut buf = vec![0u8; 2048];

    // Retry ARP a few times.
    for attempt in 1..=5 {
        nic.send(&arp_req)
            .with_context(|| format!("send arp request attempt {attempt}"))?;

        // Wait for reply (best-effort, blocking recv()).
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
        while std::time::Instant::now() < deadline {
            let n = match nic.recv_nonblocking(&mut buf) {
                Ok(Some(n)) => n,
                Ok(None) => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Err(_) => continue,
            };
            if let Ok(Some((ip, mac))) = parse_arp_reply(&buf[..n]) {
                if ip == server_ip {
                    dst_mac = Some(mac);
                    break;
                }
            }
        }

        if dst_mac.is_some() {
            break;
        }
    }

    let dst_mac = dst_mac.ok_or_else(|| anyhow::anyhow!("ARP resolve failed for 10.0.0.2"))?;
    eprintln!(
        "arp ok: {}.{}.{}.{} is {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        server_ip.0[0],
        server_ip.0[1],
        server_ip.0[2],
        server_ip.0[3],
        dst_mac.0[0],
        dst_mac.0[1],
        dst_mac.0[2],
        dst_mac.0[3],
        dst_mac.0[4],
        dst_mac.0[5]
    );

    // --- 2) Send UDP echo request (LayerInstance build) ---
    let app_payload = b"hello-echo";

    let layers = vec![
        LayerInstance {
            id: LayerId::Ether,
            inner: Box::new(ntx::network::stack::layers::Ether {
                dst: dst_mac,
                src: src_mac,
                ethertype: ntx::network::ETH_TYPE_IPV4,
            }),
        },
        LayerInstance {
            id: LayerId::Ipv4,
            inner: Box::new(ntx::network::stack::layers::Ipv4 {
                src: client_ip,
                dst: server_ip,
                proto: 17,
                ttl: 64,
                identification: 0,
                flags_fragment: 0,
                ihl_bytes: 20,
            }),
        },
        LayerInstance {
            id: LayerId::Udp,
            inner: Box::new(ntx::network::stack::layers::Udp {
                src_port: 40000,
                dst_port: port,
                src_ip: None,
                dst_ip: None,
            }),
        },
    ];

    let frame = ntx::network::stack::build_packet_with_glue(&layers, app_payload, &reg)
        .map_err(anyhow::Error::msg)
        .context("build ether/ipv4/udp with payload")?;

    nic.send(&frame).context("send udp echo request")?;
    eprintln!("sent udp echo request: len={}", frame.len());

    // --- 3) Wait for reply ---
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let n = match nic.recv_nonblocking(&mut buf) {
            Ok(Some(n)) => n,
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            Err(_) => continue,
        };

        let (layers, payload) = match parse_packet(&buf[..n], LayerId::Ether, &reg) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Filter: need Ether+Ipv4+Udp, dst mac is us, src ip is server.
        let eth = layers
            .iter()
            .find(|l| l.id == LayerId::Ether)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ether>());
        let ip = layers
            .iter()
            .find(|l| l.id == LayerId::Ipv4)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ipv4>());
        let udp = layers
            .iter()
            .find(|l| l.id == LayerId::Udp)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Udp>());

        let (Some(eth), Some(ip), Some(udp)) = (eth, ip, udp) else {
            continue;
        };
        if eth.dst != src_mac {
            continue;
        }
        if ip.src != server_ip {
            continue;
        }
        if udp.src_port != port || udp.dst_port != 40000 {
            continue;
        }

        eprintln!("got udp echo reply: {} bytes: {:?}", payload.len(), payload);
        return Ok(());
    }

    anyhow::bail!("timeout waiting for echo reply");
}
