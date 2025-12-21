use anyhow::{Context, Result};

use ntx::network::ConnTableConfig;
use ntx::network::packet::headers::{Ipv4Addr, MacAddr};
use ntx::network::prelude::*;
use ntx::network::resources::ResourcePoolsConfig;
use ntx::network::socket::{LocalIdentity, TimeContext, UdpRxContext};
use ntx::network::stack::{
    LayerId, LayerRegistry, PacketContext, default_registry, layers, parse_packet_with_ctx,
};

#[derive(Debug, Clone, serde::Deserialize)]
struct TargetsYaml {
    server: TargetsServer,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TargetsServer {
    targets: Vec<TargetItem>,
    #[allow(dead_code)]
    udp_port: u16,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TargetItem {
    #[allow(dead_code)]
    ip: [u8; 4],
    #[allow(dead_code)]
    mac: [u8; 6],
}

fn env_debug_enabled() -> bool {
    match std::env::var("NTX_ECHO_DEBUG") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v.is_empty() || v == "0" || v == "false" || v == "no")
        }
        Err(_) => false,
    }
}

/// Userspace echo server on top of AF_PACKET.
///
/// - iface: ntx1 (in netns ntxns1)
/// - IP:    one or more identities (from resources.yaml; default scenario uses 10.0.0.2 and 10.0.0.3)
/// - UDP:   port 7 (echo)
///
/// Handles:
/// - ARP request for 10.0.0.2 -> ARP reply
/// - IPv4/UDP dst_port=7 -> echo reply (swap MAC/IP/ports)
fn main() -> Result<()> {
    let debug = env_debug_enabled();

    let iface = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ntx1".to_string());

    // Optional: provide a resource pool YAML file as argv[2].
    // If omitted, we keep the historical fixed server identity (10.0.0.2 + iface MAC).
    let resources_yaml = std::env::args().nth(2);

    // Optional: provide a targets yaml as argv[3]. If present, we acquire as many identities
    // as targets (to keep the old multi-identity behavior). If absent, default to 1 identity.
    let targets_yaml = std::env::args().nth(3);

    // Fixed topology from scripts/ntx-veth-up.sh
    let _fallback_server_ip = Ipv4Addr([10, 0, 0, 2]);
    let port: u16 = 7;

    let mut nic: Box<dyn Nic> =
        Box::new(ntx::network::AfPacketNic::open(&iface).context("open afpacket nic")?);

    let iface_mac = nic
        .iface_mac()
        .context("failed to query iface mac (SIOCGIFHWADDR); are you root?")?;
    let _iface_mac = MacAddr(iface_mac);

    // Server identities (ip, mac): optionally from resources.yaml.
    let mut server_identities: Vec<(Ipv4Addr, MacAddr)> = Vec::new();
    if let Some(path) = resources_yaml.clone() {
        eprintln!("loading resource pools from: {}", path);
        let cfg = ResourcePoolsConfig::load_yaml_file(path)?;
        let mut pools = cfg.build()?;

        let identity_count = if let Some(path) = targets_yaml.as_deref() {
            let yaml = std::fs::read_to_string(path)
                .with_context(|| format!("read targets yaml: {path}"))?;
            let t: TargetsYaml = serde_yaml::from_str(&yaml)
                .with_context(|| format!("parse targets yaml: {path}"))?;
            std::cmp::max(1, t.server.targets.len())
        } else {
            1
        };

        for i in 0..identity_count {
            // In this example we don't need control-plane realism (no rids, no pinning).
            // We still avoid reaching into pool internals; only use the public acquire API.
            let ipv4_pool_candidates: [&str; 3] = ["server", "demo", "default"];
            let mac_pool_candidates: [&str; 3] = ["server", "demo", "default"];

            // Create a throwaway owner for each identity.
            let owner = pools.acquire_socket_owner(format!("ntx-echo-server-id#{i}"));

            let (_rid, v) = ipv4_pool_candidates
                .iter()
                .find_map(|pool| {
                    pools
                        .acquire_and_pin_non_socket(
                            ntx_network::resources::ResourceKind::Ipv4,
                            pool,
                            owner,
                        )
                        .ok()
                })
                .ok_or_else(|| anyhow::anyhow!("missing ipv4 pool named server/demo/default"))
                .with_context(|| format!("acquire server ipv4 identity #{i}"))?;
            let ntx_network::resources::NonSocketResourceValue::Ipv4(ip_std) = v else {
                unreachable!("resource kind/value mismatch")
            };
            let ip = Ipv4Addr(ip_std.octets());

            let (_rid, v) = mac_pool_candidates
                .iter()
                .find_map(|pool| {
                    pools
                        .acquire_and_pin_non_socket(
                            ntx_network::resources::ResourceKind::Mac,
                            pool,
                            owner,
                        )
                        .ok()
                })
                .ok_or_else(|| anyhow::anyhow!("missing mac pool named server/demo/default"))
                .with_context(|| format!("acquire server mac identity #{i}"))?;
            let ntx_network::resources::NonSocketResourceValue::Mac(mac) = v else {
                unreachable!("resource kind/value mismatch")
            };

            server_identities.push((ip, mac));
        }
    } else {
        // Historical fallback (kept for reference; scripts always pass server.yaml today).
        server_identities.push((_fallback_server_ip, _iface_mac));
    }

    // For logging / ARP reply selection fallback.
    let (server_ip, server_mac) = server_identities[0];

    eprintln!(
        "ntx-echo-server: iface={} ifindex={} ip={} mac={} udp_port={} (sudo required)",
        nic.ifname(),
        nic.ifindex(),
        ntx::network::fmt_ipv4!(server_ip),
        ntx::network::fmt_mac!(server_mac),
        port
    );

    let reg: LayerRegistry = default_registry();

    // Publish the server's receive bindings (owned IPv4s + UDP port 7) into ABR.
    // This keeps the `accept()` fast-path working end-to-end for Ether/IPv4/UDP.
    //
    // Note: ABR is process-local; server and client run as different processes so they
    // don't interfere.
    {
        let mut store = ntx::network::abr::BindingStore::default();
        let owner = ntx::network::abr::BindingOwner::Process {
            pid: std::process::id(),
        };

        for (ip, _) in server_identities.iter().copied() {
            store.add(ntx::network::abr::Binding::ipv4_be(
                u32::from_be_bytes(ip.octets()),
                owner,
            ));
        }

        // Bind UDP echo port for all owned IPs.
        // (We could also wildcard-ip bind; either works for the server side.)
        for (ip, _) in server_identities.iter().copied() {
            store.add(ntx::network::abr::Binding::udp_port_be(
                u32::from_be_bytes(ip.octets()),
                port,
                owner,
            ));
        }

        ntx::network::abr::store_view(store.snapshot());
    }

    let mut rx_arp: u64 = 0;
    let mut tx_arp: u64 = 0;
    let mut rx_udp: u64 = 0;
    let mut tx_udp: u64 = 0;
    let mut last_stats = std::time::Instant::now();

    // Socket table for UDP flows.
    // This replaces the ad-hoc HashMap<UdpReplyTemplate> and unifies recv+reply behavior.
    let mut udp_sockets = ntx::network::socket::udp::Table::new(ConnTableConfig {
        max_entries: 4096,
        ttl: Some(std::time::Duration::from_secs(60)),
    });

    let mut buf = vec![0u8; 2048];

    // Reusable per-packet context (updated each loop iteration).
    let mut ctx = PacketContext {
        // Receive frames for multiple local MACs.
        iface_mac: None,
        abr: None,
    };

    loop {
        let n = match nic.recv(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("recv error: {e:#}");
                continue;
            }
        };
        let frame = &buf[..n];

        // Refresh ABR snapshot: UDP accept() relies on it for (dst_ip, dst_port) filtering.
        ctx.abr = Some(ntx::network::abr::load_view());

        // Try decode chain using the new runtime layers + accept()-based filtering.
        let (layers, payload) = match parse_packet_with_ctx(frame, LayerId::Ether, &reg, &ctx) {
            Ok(v) => v,
            Err(e) => {
                if debug {
                    eprintln!("[dbg][rx] drop: parse failed: {e}");
                }
                continue;
            }
        };

        // Socket tables want a PacketView; `ParsedPacket` implements it.
        let pkt = ntx::network::stack::ParsedPacket { layers, payload };

        let eth_l2 = pkt
            .layers()
            .iter()
            .find(|l| l.id == LayerId::Ether)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ether>());

        // ---- ARP request for our IP -> reply ----
        if pkt.layers().iter().any(|l| l.id == LayerId::Arp) {
            let arp = pkt
                .layers()
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

                if arp.oper == 1 {
                    // If request targets one of our IPs, reply using that identity's MAC/IP.
                    let Some((my_ip, my_mac)) = server_identities
                        .iter()
                        .copied()
                        .find(|(ip, _mac)| *ip == arp.tpa)
                    else {
                        continue;
                    };

                    let reply = layers::Ether {
                        dst: arp.sha,
                        src: my_mac,
                        ethertype: ntx::network::ETH_TYPE_ARP,
                    }
                    .pkt()
                    .arp(layers::Arp {
                        oper: 2,
                        sha: my_mac,
                        spa: my_ip,
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
                        ntx::network::fmt_ipv4!(my_ip),
                        ntx::network::fmt_mac!(my_mac),
                        tx_arp,
                    );
                }
            }
            continue;
        }

        // ---- IPv4/UDP echo ----
        let eth = pkt
            .layers()
            .iter()
            .find(|l| l.id == LayerId::Ether)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ether>());
        let ip = pkt
            .layers()
            .iter()
            .find(|l| l.id == LayerId::Ipv4)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ipv4>());
        let udp = pkt
            .layers()
            .iter()
            .find(|l| l.id == LayerId::Udp)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Udp>());

        let (Some(_eth), Some(ip), Some(udp)) = (eth, ip, udp) else {
            if debug {
                eprintln!("[dbg][udp] drop: missing ether/ipv4/udp layers");
            }
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

        // Choose which server identity to reply from based on the dst IP of the request.
        let Some((my_ip, my_mac)) = server_identities
            .iter()
            .copied()
            .find(|(sip, _mac)| *sip == ip.dst)
        else {
            if debug {
                eprintln!(
                    "[dbg][udp] drop: dst_ip not owned: dst_ip={} (identities={:?})",
                    ntx::network::fmt_ipv4!(ip.dst),
                    server_identities
                        .iter()
                        .map(|(i, _)| ntx::network::fmt_ipv4!(*i).to_string())
                        .collect::<Vec<_>>()
                );
            }
            continue;
        };

        // Use the unified UDP socket table to track flows and build replies.
        // We must provide the local identity chosen above so the socket layer knows
        // which MAC/IP/port we are receiving for.
        let udp_ctx =
            UdpRxContext::new(TimeContext::new(), LocalIdentity::new(my_mac, my_ip), port);

        let conn = udp_sockets.on_rx(&pkt, &udp_ctx).context("udp on_rx")?;
        let reply = conn.build_reply(payload).context("build udp reply")?.bytes;

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
