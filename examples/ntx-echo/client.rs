use anyhow::{Context, Result};

use ntx_network::packet::headers::{Ipv4Addr, MacAddr};
use ntx_network::prelude::*;
use ntx_network::resources::ResourcePoolsConfig;
use ntx_network::socket::udp::{Key, Table};
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

    // Client-side connection table + ARP cache.
    // - ArpCache learns (peer_ip -> peer_mac) from ARP replies.
    // - Table remembers per-flow peer/local tuples and holds a reusable tx template.
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
    for i in 0..10 {
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

    // --- 3) Send UDP echo requests: 10 client identities x N targets ---
    let total_expected = identities.len() * resolved_targets.len();
    for (cidx, (client_ip, src_mac, udp_src_port)) in identities.iter().copied().enumerate() {
        for (tidx, (server_ip, dst_mac, server_port)) in
            resolved_targets.iter().copied().enumerate()
        {
            let app_payload = format!("hello-echo-c{:02}-t{:02}", cidx, tidx);

            // Ensure we have a connected socket for this (peer, local) tuple.
            // This uses ARP cache to resolve peer MAC and stores a reusable tx template.
            let sock = udp_sockets
                .connect_via_arp_cache(
                    &mut arp_cache,
                    server_ip,
                    server_port,
                    client_ip,
                    udp_src_port,
                    src_mac,
                    64,
                )
                .with_context(|| {
                    format!(
                        "connect via arp cache: peer={} local={} src_port={udp_src_port}",
                        ntx::network::fmt_ipv4!(server_ip),
                        ntx::network::fmt_ipv4!(client_ip)
                    )
                })?;

            // Use the socket template to build the bytes. (Despite the name `reply`,
            // the template encode path is the same and includes checksums.)
            let frame = sock
                .build_reply(app_payload.as_bytes())
                .with_context(|| format!("build echo request (socket) c={cidx} t={tidx}"))?
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

    // --- 4) Wait for replies (expect up to 10*N) ---
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

        let ip = layers
            .iter()
            .find(|l| l.id == LayerId::Ipv4)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ipv4>());
        let udp = layers
            .iter()
            .find(|l| l.id == LayerId::Udp)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Udp>());

        let (Some(ip), Some(udp)) = (ip, udp) else {
            if debug {
                eprintln!("[dbg][rx] drop: missing ip/udp layer");
            }
            continue;
        };

        // Correlate using the unified socket-table key instead of ad-hoc tuple matching.
        // Incoming reply should match an existing connected socket:
        // - peer_ip == reply src
        // - local_ip == reply dst
        // - peer_port == reply src_port
        // - local_port == reply dst_port
        let flow_key = Key {
            peer_ip: ip.src,
            peer_port: udp.src_port,
            local_ip: ip.dst,
            local_port: udp.dst_port,
        };
        if udp_sockets.get(&flow_key).is_none() {
            if debug {
                eprintln!(
                    "[dbg][rx] drop: no matching udp socket: peer={} local={} ports {}->{}",
                    ntx::network::fmt_ipv4!(ip.src),
                    ntx::network::fmt_ipv4!(ip.dst),
                    udp.src_port,
                    udp.dst_port
                );
            }
            continue;
        }

        // Keep old reporting shape (cidx,tidx) by mapping using identities + resolved targets.
        // This preserves the existing success criteria (10 x N replies).
        let Some(cidx) = identities
            .iter()
            .position(|(client_ip, _mac, udp_src_port)| {
                *client_ip == ip.dst && *udp_src_port == udp.dst_port
            })
        else {
            continue;
        };
        let Some(tidx) = resolved_targets
            .iter()
            .position(|(server_ip, _mac, _port)| *server_ip == ip.src)
        else {
            continue;
        };

        if got.insert((cidx, tidx)) {
            eprintln!(
                "got reply c#{cidx} <- t#{tidx}: dst_ip={} dst_port={} {} bytes: {:?}",
                ntx::network::fmt_ipv4!(ip.dst),
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
