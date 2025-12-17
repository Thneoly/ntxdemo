use anyhow::{Context, Result};

use ntx::network::Nic;
use ntx::network::packet::headers::{Ipv4Addr, MacAddr};
use ntx::network::stack::{LayerId, LayerInstance, LayerRegistry, default_registry, parse_packet};

/// Userspace echo server on top of AF_PACKET.
///
/// - iface: ntx1 (in netns ntxns1)
/// - IP:    10.0.0.2
/// - UDP:   port 7 (echo)
///
/// Handles:
/// - ARP request for 10.0.0.2 -> ARP reply
/// - IPv4/UDP dst_port=7 -> echo reply (swap MAC/IP/ports)
fn main() -> Result<()> {
    let iface = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ntx1".to_string());

    // Fixed topology from scripts/ntx-veth-up.sh
    let server_ip = Ipv4Addr([10, 0, 0, 2]);
    let port: u16 = 7;

    let mut nic: Box<dyn Nic> =
        Box::new(ntx::network::AfPacketNic::open(&iface).context("open afpacket nic")?);

    let iface_mac = nic
        .iface_mac()
        .context("failed to query iface mac (SIOCGIFHWADDR); are you root?")?;
    let server_mac = MacAddr(iface_mac);

    eprintln!(
        "ntx-echo-server: iface={} ifindex={} ip={}.{}.{}.{} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} udp_port={} (sudo required)",
        nic.ifname(),
        nic.ifindex(),
        server_ip.0[0],
        server_ip.0[1],
        server_ip.0[2],
        server_ip.0[3],
        server_mac.0[0],
        server_mac.0[1],
        server_mac.0[2],
        server_mac.0[3],
        server_mac.0[4],
        server_mac.0[5],
        port
    );

    let reg: LayerRegistry = default_registry();

    let mut rx_arp: u64 = 0;
    let mut tx_arp: u64 = 0;
    let mut rx_udp: u64 = 0;
    let mut tx_udp: u64 = 0;
    let mut last_stats = std::time::Instant::now();

    let mut buf = vec![0u8; 2048];

    loop {
        let n = match nic.recv(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("recv error: {e:#}");
                continue;
            }
        };
        let frame = &buf[..n];

        // Try decode chain using the new runtime layers.
        let (layers, payload) = match parse_packet(frame, LayerId::Ether, &reg) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let eth_l2 = layers
            .iter()
            .find(|l| l.id == LayerId::Ether)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ether>());

        // ---- ARP request for our IP -> reply ----
        if layers.iter().any(|l| l.id == LayerId::Arp) {
            let arp = layers
                .iter()
                .find(|l| l.id == LayerId::Arp)
                .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Arp>());

            if let Some(arp) = arp {
                rx_arp += 1;
                if let Some(eth) = eth_l2 {
                    eprintln!(
                        "[arp rx] l2 {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  oper={} sha={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} spa={}.{}.{}.{} tha={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} tpa={}.{}.{}.{}",
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
                        arp.oper,
                        arp.sha.0[0],
                        arp.sha.0[1],
                        arp.sha.0[2],
                        arp.sha.0[3],
                        arp.sha.0[4],
                        arp.sha.0[5],
                        arp.spa.0[0],
                        arp.spa.0[1],
                        arp.spa.0[2],
                        arp.spa.0[3],
                        arp.tha.0[0],
                        arp.tha.0[1],
                        arp.tha.0[2],
                        arp.tha.0[3],
                        arp.tha.0[4],
                        arp.tha.0[5],
                        arp.tpa.0[0],
                        arp.tpa.0[1],
                        arp.tpa.0[2],
                        arp.tpa.0[3],
                    );
                } else {
                    eprintln!(
                        "[arp rx] oper={} sha={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} spa={}.{}.{}.{} tha={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} tpa={}.{}.{}.{}",
                        arp.oper,
                        arp.sha.0[0],
                        arp.sha.0[1],
                        arp.sha.0[2],
                        arp.sha.0[3],
                        arp.sha.0[4],
                        arp.sha.0[5],
                        arp.spa.0[0],
                        arp.spa.0[1],
                        arp.spa.0[2],
                        arp.spa.0[3],
                        arp.tha.0[0],
                        arp.tha.0[1],
                        arp.tha.0[2],
                        arp.tha.0[3],
                        arp.tha.0[4],
                        arp.tha.0[5],
                        arp.tpa.0[0],
                        arp.tpa.0[1],
                        arp.tpa.0[2],
                        arp.tpa.0[3],
                    );
                }

                if arp.tpa == server_ip && arp.oper == 1 {
                    let eth = LayerInstance {
                        id: LayerId::Ether,
                        inner: Box::new(ntx::network::stack::layers::Ether {
                            dst: arp.sha,
                            src: server_mac,
                            ethertype: ntx::network::ETH_TYPE_ARP,
                        }),
                    };
                    let arp_reply = LayerInstance {
                        id: LayerId::Arp,
                        inner: Box::new(ntx::network::stack::layers::Arp {
                            oper: 2,
                            sha: server_mac,
                            spa: server_ip,
                            tha: arp.sha,
                            tpa: arp.spa,
                        }),
                    };
                    let reply =
                        ntx::network::stack::build_packet_no_payload(&[eth, arp_reply], &reg)
                            .map_err(anyhow::Error::msg)
                            .context("build arp reply")?;
                    let _ = nic.send(&reply);
                    tx_arp += 1;
                    eprintln!(
                        "[arp tx] to {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  {}.{}.{}.{} is-at {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} (tx_arp={})",
                        arp.sha.0[0],
                        arp.sha.0[1],
                        arp.sha.0[2],
                        arp.sha.0[3],
                        arp.sha.0[4],
                        arp.sha.0[5],
                        server_ip.0[0],
                        server_ip.0[1],
                        server_ip.0[2],
                        server_ip.0[3],
                        server_mac.0[0],
                        server_mac.0[1],
                        server_mac.0[2],
                        server_mac.0[3],
                        server_mac.0[4],
                        server_mac.0[5],
                        tx_arp,
                    );
                }
            }
            continue;
        }

        // ---- IPv4/UDP echo ----
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
        if eth.dst != server_mac {
            continue;
        }
        if ip.dst != server_ip {
            continue;
        }
        if udp.dst_port != port {
            continue;
        }

        rx_udp += 1;
        eprintln!(
            "[udp rx] {}.{}.{}.{}:{} -> {}.{}.{}.{}:{}  len={} (rx_udp={}, tx_udp={})",
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
            payload.len(),
            rx_udp,
            tx_udp,
        );

        // Echo reply: swap L2/L3/L4 src/dst and copy payload.
        let reply_layers = vec![
            LayerInstance {
                id: LayerId::Ether,
                inner: Box::new(ntx::network::stack::layers::Ether {
                    dst: eth.src,
                    src: server_mac,
                    ethertype: ntx::network::ETH_TYPE_IPV4,
                }),
            },
            LayerInstance {
                id: LayerId::Ipv4,
                inner: Box::new(ntx::network::stack::layers::Ipv4 {
                    src: server_ip,
                    dst: ip.src,
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
                    src_port: port,
                    dst_port: udp.src_port,
                    src_ip: None,
                    dst_ip: None,
                }),
            },
        ];

        let reply = ntx::network::stack::build_packet_with_glue(&reply_layers, payload, &reg)
            .map_err(anyhow::Error::msg)
            .context("build udp reply")?;

        let _ = nic.send(&reply);
        tx_udp += 1;

        if last_stats.elapsed() >= std::time::Duration::from_secs(2) {
            eprintln!(
                "[stats] rx_arp={} tx_arp={} rx_udp={} tx_udp={}",
                rx_arp, tx_arp, rx_udp, tx_udp
            );
            last_stats = std::time::Instant::now();
        }
    }
}
