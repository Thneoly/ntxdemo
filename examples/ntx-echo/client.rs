use anyhow::{Context, Result};

use ntx_network::packet::headers::{Ipv4Addr, MacAddr};
use ntx_network::prelude::*;
use ntx_network::resources::ResourcePoolsConfig;
use ntx_network::socket::udp::{Table, UdpBinding};
use ntx_network::socket::{LocalIdentity, TimeContext, UdpRxContext};
use ntx_network::stack::{
    LayerId, LayerRegistry, PacketContext, default_registry, layers, li, parse_packet_with_ctx,
};
use ntx_network::{ArpCache, ConnTableConfig};

#[derive(Debug, Clone, serde::Deserialize)]
struct TargetsConfig {
    server: ServerTargets,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ServerTargets {
    udp_port: u16,
    targets: Vec<TargetItem>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
struct TargetItem {
    ip: [u8; 4],
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
    let debug = env_debug_enabled();

    let iface = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ntx0".to_string());

    // Client resource pools YAML path as argv[2].
    let resources_yaml = std::env::args().nth(2);

    // Optional: targets YAML path as argv[3].
    // If omitted, fall back to a single classic target (10.0.0.2:7) and ARP resolve its MAC.
    let targets_yaml = std::env::args().nth(3);

    let port: u16 = 7;

    let mut nic: Box<dyn Nic> =
        Box::new(ntx::network::AfPacketNic::open(&iface).context("open afpacket nic")?);

    let iface_mac = nic
        .iface_mac()
        .context("failed to query iface mac (SIOCGIFHWADDR); are you root?")?;
    // NOTE: iface MAC is still used for ARP resolution. Actual transmit frames will use
    // per-request (ip,mac) identities from resource pools.
    let iface_mac = MacAddr(iface_mac);

    eprintln!(
        "ntx-echo-client: iface={} ifindex={} (sudo required)",
        nic.ifname(),
        nic.ifindex(),
    );

    let reg: LayerRegistry = default_registry();

    // Client-side UDP socket table + ARP cache.
    // - ArpCache learns (peer_ip -> peer_mac) from ARP replies.
    // - Table stores per-flow socket bindings and can build tx frames by sock_id.
    let mut arp_cache = ArpCache::new(std::time::Duration::from_secs(60));
    let mut udp_sockets = Table::new(ConnTableConfig {
        max_entries: 4096,
        ttl: Some(std::time::Duration::from_secs(60)),
    });

    // --- Resource pools: allocate 10 identities (ip, mac, udp-port) ---
    let mut pools = if let Some(path) = resources_yaml {
        eprintln!("loading resource pools from: {}", path);
        let cfg = ResourcePoolsConfig::load_yaml_file(path)?;
        cfg.build()?
    } else {
        panic!("missing client resource pools YAML file argument");
    };

    let mut identities: Vec<(Ipv4Addr, MacAddr, u16)> = Vec::with_capacity(10);
    for i in 0..1 {
        let ip = {
            let pool = if let Some(p) = pools.ipv4("client") {
                p
            } else if let Some(p) = pools.ipv4("demo") {
                p
            } else {
                pools
                    .ipv4("default")
                    .context("missing ipv4 pool named client/demo/default")?
            };
            pool.acquire()
                .ok_or_else(|| anyhow::anyhow!("ipv4 pool exhausted"))
                .with_context(|| format!("allocate ipv4 identity #{i}"))?
        };

        let mac = {
            let pool = if let Some(p) = pools.mac("client") {
                p
            } else if let Some(p) = pools.mac("demo") {
                p
            } else {
                pools
                    .mac("default")
                    .context("missing mac pool named client/demo/default")?
            };
            pool.acquire()
                .ok_or_else(|| anyhow::anyhow!("mac pool exhausted"))
                .with_context(|| format!("allocate mac identity #{i}"))?
        };

        let udp_port = {
            let pool = if let Some(p) = pools.udp_port("client") {
                p
            } else if let Some(p) = pools.udp_port("demo") {
                p
            } else {
                pools
                    .udp_port("default")
                    .context("missing udp_port pool named client/demo/default")?
            };
            pool.acquire()
                .ok_or_else(|| anyhow::anyhow!("udp port pool exhausted"))
                .with_context(|| format!("allocate udp port identity #{i}"))?
        };

        identities.push((ip, mac, udp_port));
    }

    eprintln!("allocated {} identities:", identities.len());
    for (idx, (ip, mac, udp_port)) in identities.iter().enumerate() {
        eprintln!(
            "  #{idx}: ip={} mac={} udp_src_port={udp_port}",
            ntx::network::fmt_ipv4!(*ip),
            ntx::network::fmt_mac!(*mac)
        );
    }

    let mut buf = vec![0u8; 2048];

    // --- Local identity lookup: (dst_ip, dst_port) -> LocalIdentity ---
    // Built once so RX doesn't need to scan `identities` per packet.
    let local_map: std::collections::HashMap<(Ipv4Addr, u16), LocalIdentity> = identities
        .iter()
        .copied()
        .map(|(ip, mac, port)| ((ip, port), LocalIdentity::new(mac, ip)))
        .collect();

    // For logging/stats only: map inbound (dst_ip,dst_port) back to identity index.
    let cidx_map: std::collections::HashMap<(Ipv4Addr, u16), usize> = identities
        .iter()
        .enumerate()
        .map(|(idx, (ip, _mac, port))| ((*ip, *port), idx))
        .collect();

    // Reusable per-packet context (updated in polling loops).
    let mut ctx = PacketContext {
        // Client uses multiple distinct destination MACs (one per identity), not the NIC's
        // kernel MAC. Filtering on iface_mac would drop valid replies destined to those
        // identity MACs. We instead disable L2 filtering here and rely on ABR (and the
        // reply matching logic below) to keep only relevant packets.
        iface_mac: None,
        abr: None,
    };

    // Relaxed context for debug-only “what did we actually receive?” decoding.
    // Setting iface_mac=None bypasses Ether::accept filtering. This should never be used
    // for normal logic — only for printing diagnostics when enabled.
    let mut debug_ctx = PacketContext {
        iface_mac: None,
        abr: None,
    };

    // --- 1) Targets (server identities) + ARP resolve (if MAC unspecified) ---
    let targets: Vec<(Ipv4Addr, MacAddr, u16)> = if let Some(path) = targets_yaml {
        eprintln!("loading targets from: {}", path);
        let bytes = std::fs::read(&path).with_context(|| format!("read targets yaml: {path}"))?;
        let cfg: TargetsConfig = serde_yaml::from_slice(&bytes)
            .with_context(|| format!("parse targets yaml: {path}"))?;
        cfg.server
            .targets
            .iter()
            .map(|t| Ok((Ipv4Addr(t.ip), MacAddr(t.mac), cfg.server.udp_port)))
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![(Ipv4Addr([10, 0, 0, 2]), MacAddr([0, 0, 0, 0, 0, 0]), port)]
    };

    let mac_broadcast = MacAddr([0xff; 6]);
    let Some((arp_spa, _, _)) = identities.first().copied() else {
        anyhow::bail!("no client identities allocated");
    };

    let mut resolved_targets: Vec<(Ipv4Addr, MacAddr, u16)> = Vec::new();
    for (t_ip, t_mac, t_port) in targets.into_iter() {
        let dst_mac = if t_mac.0 != [0, 0, 0, 0, 0, 0] {
            t_mac
        } else {
            let arp_req_layers = [
                li::ether(layers::Ether {
                    dst: mac_broadcast,
                    src: iface_mac,
                    ethertype: ntx::network::ETH_TYPE_ARP,
                }),
                li::arp(layers::Arp {
                    oper: 1,
                    sha: iface_mac,
                    spa: arp_spa,
                    tha: MacAddr([0, 0, 0, 0, 0, 0]),
                    tpa: t_ip,
                }),
            ];

            let arp_req = ntx::network::stack::build_packet_no_payload(&arp_req_layers, &reg)
                .map_err(anyhow::Error::msg)
                .context("build arp request")?;

            let mut got_mac: Option<MacAddr> = None;
            for attempt in 1..=5 {
                nic.send(&arp_req).with_context(|| {
                    format!(
                        "send arp request for {} attempt {attempt}",
                        ntx::network::fmt_ipv4!(t_ip)
                    )
                })?;

                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
                while std::time::Instant::now() < deadline {
                    ctx.abr = Some(ntx::network::abr::load_view());

                    let n = match nic.recv_nonblocking(&mut buf) {
                        Ok(Some(n)) => n,
                        Ok(None) => {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                            continue;
                        }
                        Err(_) => continue,
                    };

                    let (layers, _payload) =
                        match parse_packet_with_ctx(&buf[..n], LayerId::Ether, &reg, &ctx) {
                            Ok(v) => v,
                            Err(e) => {
                                if debug {
                                    eprintln!("[dbg][rx] drop: parse failed: {e}");
                                }
                                continue;
                            }
                        };

                    let arp = layers
                        .iter()
                        .find(|l| l.id == LayerId::Arp)
                        .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Arp>());
                    let Some(arp) = arp else {
                        if debug {
                            eprintln!("[dbg][arp] drop: no arp layer");
                        }
                        continue;
                    };
                    if arp.oper != 2 {
                        if debug {
                            eprintln!("[dbg][arp] drop: not reply oper={}", arp.oper);
                        }
                        continue;
                    }
                    if arp.spa != t_ip {
                        if debug {
                            eprintln!(
                                "[dbg][arp] drop: spa mismatch spa={} want={}",
                                ntx::network::fmt_ipv4!(arp.spa),
                                ntx::network::fmt_ipv4!(t_ip)
                            );
                        }
                        continue;
                    }

                    got_mac = Some(arp.sha);
                    break;
                }

                if got_mac.is_some() {
                    break;
                }
            }

            got_mac.ok_or_else(|| {
                anyhow::anyhow!("ARP resolve failed for {}", ntx::network::fmt_ipv4!(t_ip))
            })?
        };

        // Learn into ARP cache so connect() can be purely cache-backed.
        arp_cache.insert(t_ip, dst_mac);

        resolved_targets.push((t_ip, dst_mac, t_port));
        eprintln!(
            "target: {} is {} udp_port={}",
            ntx::network::fmt_ipv4!(t_ip),
            ntx::network::fmt_mac!(dst_mac),
            t_port,
        );
    }

    // For RX logging/stats: map inbound server identity back to target index.
    // Key by (server_ip, server_port) so it's unambiguous even if multiple ports are used.
    let tidx_map: std::collections::HashMap<(Ipv4Addr, u16), usize> = resolved_targets
        .iter()
        .enumerate()
        .map(|(idx, (ip, _mac, port))| ((*ip, *port), idx))
        .collect();

    // --- 2) Publish ABR snapshot for accept()-based filtering (all 10 identities) ---
    // Important: for RX replies, UDP::accept checks bindings against (dst_ip, dst_port).
    // Replies are destined to each client's IP, but the bound *socket-like* concept on the
    // client is really "I want to receive on these local UDP ports".
    //
    // So we publish:
    // - all local IPv4 identities (so Ipv4::accept accepts dst_ip)
    // - wildcard-IP UDP port bindings (0.0.0.0, udp_src_port) so Udp::accept accepts replies
    //   regardless of which local IP the reply is destined to.
    let mut abr_store = ntx::network::abr::BindingStore::default();
    for (ip, _mac, udp_port) in &identities {
        let ip_be = u32::from_be_bytes(ip.octets());
        abr_store.add(ntx::network::abr::Binding::ipv4_be(
            ip_be,
            ntx::network::abr::BindingOwner::KernelIface,
        ));
        // Bind local UDP ports with wildcard IP (0.0.0.0) to match Udp::accept policy.
        abr_store.add(ntx::network::abr::Binding::udp_port_be(
            0,
            *udp_port,
            ntx::network::abr::BindingOwner::KernelIface,
        ));
    }
    ntx::network::abr::store_view(abr_store.snapshot());

    // --- 3) Create+bind one UDP socket per (client identity, target) pair ---
    // This avoids creating a fresh `sock_id` for every send and keeps the socket table bounded.
    let mut sock_ids: Vec<Vec<u64>> = vec![vec![0u64; resolved_targets.len()]; identities.len()];
    for (cidx, (client_ip, src_mac, udp_src_port)) in identities.iter().copied().enumerate() {
        for (tidx, (server_ip, dst_mac, server_port)) in
            resolved_targets.iter().copied().enumerate()
        {
            let sock_id = udp_sockets.create_sock_id();
            udp_sockets.bind_sock_id(
                sock_id,
                UdpBinding {
                    peer_ip: server_ip,
                    peer_port: server_port,
                    local_ip: client_ip,
                    local_port: udp_src_port,
                    peer_mac: dst_mac,
                    local_mac: src_mac,
                    ttl: 64,
                },
            );
            sock_ids[cidx][tidx] = sock_id;
        }
    }

    // --- 4) Send UDP echo requests: 10 client identities x N targets ---
    let total_expected = identities.len() * resolved_targets.len();
    for (cidx, (client_ip, src_mac, udp_src_port)) in identities.iter().copied().enumerate() {
        for (tidx, (server_ip, dst_mac, server_port)) in
            resolved_targets.iter().copied().enumerate()
        {
            let app_payload = format!("hello-echo-c{:02}-t{:02}", cidx, tidx);

            let sock_id = sock_ids[cidx][tidx];

            let frame = udp_sockets
                .build_reply_for_sock_id(sock_id, app_payload.as_bytes())
                .map_err(|e| anyhow::anyhow!("build udp frame: {e}"))?
                .bytes;

            // Best-effort send.
            nic.send(&frame)
                .with_context(|| format!("send udp echo request c={cidx} t={tidx}"))?;
            eprintln!(
                "sent c#{cidx} -> t#{tidx}: src_ip={} src_mac={} src_port={}  dst_ip={} dst_mac={} dst_port={} len={}",
                ntx::network::fmt_ipv4!(client_ip),
                ntx::network::fmt_mac!(src_mac),
                udp_src_port,
                ntx::network::fmt_ipv4!(server_ip),
                ntx::network::fmt_mac!(dst_mac),
                server_port,
                frame.len()
            );
        }
    }

    // --- 5) Wait for replies (expect up to 10*N) ---
    use std::collections::BTreeSet;
    let mut got: BTreeSet<(usize, usize)> = BTreeSet::new();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline && got.len() < total_expected {
        // Dataplane pattern: load a stable ABR snapshot once per polling iteration.
        ctx.abr = Some(ntx::network::abr::load_view());

        let n = match nic.recv_nonblocking(&mut buf) {
            Ok(Some(n)) => n,
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            Err(_) => continue,
        };

        let (layers, payload) = match parse_packet_with_ctx(&buf[..n], LayerId::Ether, &reg, &ctx) {
            Ok(v) => v,
            Err(e) => {
                if debug {
                    eprintln!("[dbg][rx] drop: parse failed: {e:#}");

                    // Try to decode again with a relaxed context so we can print L2/L3/L4 tuple.
                    // This is diagnostic only.
                    debug_ctx.abr = ctx.abr.clone();
                    if let Ok((dlayers, dpayload)) =
                        parse_packet_with_ctx(&buf[..n], LayerId::Ether, &reg, &debug_ctx)
                    {
                        let eth = dlayers
                            .iter()
                            .find(|l| l.id == LayerId::Ether)
                            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ether>());
                        let ip = dlayers
                            .iter()
                            .find(|l| l.id == LayerId::Ipv4)
                            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ipv4>());
                        let udp = dlayers
                            .iter()
                            .find(|l| l.id == LayerId::Udp)
                            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Udp>());

                        if let Some(eth) = eth {
                            eprintln!(
                                "[dbg][rx] l2: {} -> {} ethertype=0x{:04x}",
                                ntx::network::fmt_mac!(eth.src),
                                ntx::network::fmt_mac!(eth.dst),
                                eth.ethertype
                            );
                        }
                        if let (Some(ip), Some(udp)) = (ip, udp) {
                            eprintln!(
                                "[dbg][rx] l3/l4: {}:{} -> {}:{} payload_len={}",
                                ntx::network::fmt_ipv4!(ip.src),
                                udp.src_port,
                                ntx::network::fmt_ipv4!(ip.dst),
                                udp.dst_port,
                                dpayload.len()
                            );
                        }
                    } else {
                        eprintln!("[dbg][rx] relaxed decode also failed");
                    }
                }
                continue;
            }
        };

        let pkt = ntx::network::stack::ParsedPacket {
            layers,
            payload: &payload,
        };

        // Attribute replies by (dst_ip, dst_port) using the prebuilt map.
        // This avoids scanning `identities` on every RX packet.
        let Some(udp_ctx) =
            UdpRxContext::from_ipv4_udp_packet(&pkt, TimeContext::new(), &local_map)
        else {
            if debug {
                eprintln!("[dbg][rx] drop: dst not one of our identities");
            }
            continue;
        };

        // For logging + tidx mapping we still need the IP/UDP layers (cheap, no scan).
        let Some(ip_layer) = pkt.get::<ntx::network::stack::layers::Ipv4>() else {
            continue;
        };
        let Some(udp) = pkt.get::<ntx::network::stack::layers::Udp>() else {
            continue;
        };

        // Keep old reporting shape (cidx,tidx) for printing/stats.
        let Some(&cidx) = cidx_map.get(&(ip_layer.dst, udp.dst_port)) else {
            continue;
        };

        // If parsing reached UDP, maintain/refresh the socket entry.
        let _ = udp_sockets.on_rx(&pkt, &udp_ctx);

        // Keep old reporting shape (cidx,tidx) for printing/stats.
        let Some(&tidx) = tidx_map.get(&(ip_layer.src, udp.src_port)) else {
            continue;
        };

        if got.insert((cidx, tidx)) {
            eprintln!(
                "got reply c#{cidx} <- t#{tidx}: dst_ip={} dst_port={} {} bytes: {:?}",
                ntx::network::fmt_ipv4!(ip_layer.dst),
                udp.dst_port,
                payload.len(),
                payload
            );
        }
    }

    if got.len() == total_expected {
        Ok(())
    } else {
        anyhow::bail!(
            "timeout waiting for echo replies: got {}/{}",
            got.len(),
            total_expected
        )
    }
}
