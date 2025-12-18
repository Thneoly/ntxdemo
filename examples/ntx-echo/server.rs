use anyhow::{Context, Result};

use ntx::network::packet::headers::{Ipv4Addr, MacAddr};
use ntx::network::prelude::*;
use ntx::network::stack::{
    LayerId, LayerRegistry, PacketContext, Raw, default_registry, layers, parse_packet_with_ctx,
};

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
        "ntx-echo-server: iface={} ifindex={} ip={} mac={} udp_port={} (sudo required)",
        nic.ifname(),
        nic.ifindex(),
        ntx::network::fmt_ipv4!(server_ip),
        ntx::network::fmt_mac!(server_mac),
        port
    );

    let reg: LayerRegistry = default_registry();

    // Publish ABR snapshot for accept()-based filtering.
    // This example is single-threaded, so the simplest approach is to publish once.
    // (In a real control plane, you'd periodically reconcile and publish updates.)
    let mut abr_store = ntx::network::abr::BindingStore::default();
    abr_store.add(ntx::network::abr::Binding::ipv4_be(
        u32::from_be_bytes(server_ip.octets()),
        ntx::network::abr::BindingOwner::KernelIface,
    ));
    abr_store.add(ntx::network::abr::Binding::udp_port_be(
        u32::from_be_bytes(server_ip.octets()),
        port,
        ntx::network::abr::BindingOwner::KernelIface,
    ));
    ntx::network::abr::store_view(abr_store.snapshot());

    let mut rx_arp: u64 = 0;
    let mut tx_arp: u64 = 0;
    let mut rx_udp: u64 = 0;
    let mut tx_udp: u64 = 0;
    let mut last_stats = std::time::Instant::now();

    let mut buf = vec![0u8; 2048];

    // Reusable per-packet context (updated each loop iteration).
    let mut ctx = PacketContext {
        iface_mac: Some(server_mac),
        abr: None,
        local_ipv4: Vec::new(),
    };

    loop {
        // Dataplane pattern: load a stable ABR snapshot once per loop iteration.
        ctx.abr = Some(ntx::network::abr::load_view());

        let n = match nic.recv(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("recv error: {e:#}");
                continue;
            }
        };
        let frame = &buf[..n];

        // Try decode chain using the new runtime layers + accept()-based filtering.
        let (layers, payload) = match parse_packet_with_ctx(frame, LayerId::Ether, &reg, &ctx) {
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
                    eprintln!("[arp rx] {}", ntx::network::fmt_ether_arp!(eth, arp));
                } else {
                    eprintln!("[arp rx] {}", ntx::network::fmt_arp!(arp));
                }

                if arp.tpa == server_ip && arp.oper == 1 {
                    let reply = layers::Ether {
                        dst: arp.sha,
                        src: server_mac,
                        ethertype: ntx::network::ETH_TYPE_ARP,
                    }
                    .pkt()
                    .arp(layers::Arp {
                        oper: 2,
                        sha: server_mac,
                        spa: server_ip,
                        tha: arp.sha,
                        tpa: arp.spa,
                    })
                    .build(&reg)
                    .map_err(anyhow::Error::msg)
                    .context("build arp reply")?;
                    let _ = nic.send(&reply);
                    tx_arp += 1;
                    eprintln!(
                        "[arp tx] to {}  {} is-at {} (tx_arp={})",
                        ntx::network::fmt_mac!(arp.sha),
                        ntx::network::fmt_ipv4!(server_ip),
                        ntx::network::fmt_mac!(server_mac),
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
        // Note: Ether/Arp/Udp layers already applied accept() checks via ABR+ctx.

        rx_udp += 1;
        eprintln!(
            "[udp rx] {}:{} -> {}:{}  len={} (rx_udp={}, tx_udp={})",
            ntx::network::fmt_ipv4!(ip.src),
            udp.src_port,
            ntx::network::fmt_ipv4!(ip.dst),
            udp.dst_port,
            payload.len(),
            rx_udp,
            tx_udp,
        );

        // Echo reply: swap L2/L3/L4 src/dst and copy payload.
        let reply = layers::Ether {
            dst: eth.src,
            src: server_mac,
            ethertype: ntx::network::ETH_TYPE_IPV4,
        }
        .pkt()
        .ipv4(layers::Ipv4 {
            src: server_ip,
            dst: ip.src,
            proto: 17,
            ttl: 64,
            identification: 0,
            flags_fragment: 0,
            ihl_bytes: 20,
        })
        .udp(layers::Udp {
            src_port: port,
            dst_port: udp.src_port,
            src_ip: None,
            dst_ip: None,
        })
        .raw(Raw::new(payload))
        .build(&reg)
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
