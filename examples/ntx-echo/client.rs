use anyhow::{Context, Result};

use ntx::network::packet::headers::{Ipv4Addr, MacAddr};
use ntx::network::prelude::*;
use ntx::network::stack::{
    LayerId, LayerRegistry, PacketContext, Raw, default_registry, layers, li, parse_packet_with_ctx,
};

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
        "ntx-echo-client: iface={} ifindex={} ip={} mac={} target={}:{} (sudo required)",
        nic.ifname(),
        nic.ifindex(),
        ntx::network::fmt_ipv4!(client_ip),
        ntx::network::fmt_mac!(src_mac),
        ntx::network::fmt_ipv4!(server_ip),
        port
    );

    let reg: LayerRegistry = default_registry();

    // Publish ABR snapshot for accept()-based filtering.
    // We bind our own IP/MAC + the client UDP port so only relevant replies get parsed.
    let mut abr_store = ntx::network::abr::BindingStore::default();
    abr_store.add(ntx::network::abr::Binding::ipv4_be(
        u32::from_be_bytes(client_ip.octets()),
        ntx::network::abr::BindingOwner::KernelIface,
    ));
    abr_store.add(ntx::network::abr::Binding::udp_port_be(
        u32::from_be_bytes(client_ip.octets()),
        40000,
        ntx::network::abr::BindingOwner::KernelIface,
    ));
    ntx::network::abr::store_view(abr_store.snapshot());

    // --- 1) ARP resolve ---
    let mut dst_mac: Option<MacAddr> = None;
    let mac_broadcast = MacAddr([0xff; 6]);
    let arp_req_layers = [
        li::ether(layers::Ether {
            dst: mac_broadcast,
            src: src_mac,
            ethertype: ntx::network::ETH_TYPE_ARP,
        }),
        li::arp(layers::Arp {
            oper: 1,
            sha: src_mac,
            spa: client_ip,
            tha: MacAddr([0, 0, 0, 0, 0, 0]),
            tpa: server_ip,
        }),
    ];
    let arp_req = ntx::network::stack::build_packet_no_payload(&arp_req_layers, &reg)
        .map_err(anyhow::Error::msg)
        .context("build arp request")?;
    let mut buf = vec![0u8; 2048];

    // Reusable per-packet context (updated in polling loops).
    let mut ctx = PacketContext {
        iface_mac: Some(src_mac),
        abr: None,
        local_ipv4: Vec::new(),
    };

    // Retry ARP a few times.
    for attempt in 1..=5 {
        nic.send(&arp_req)
            .with_context(|| format!("send arp request attempt {attempt}"))?;

        // Wait for reply (best-effort, blocking recv()).
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
        while std::time::Instant::now() < deadline {
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

            let (layers, _payload) =
                match parse_packet_with_ctx(&buf[..n], LayerId::Ether, &reg, &ctx) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

            // We only care about ARP replies for server_ip.
            let arp = layers
                .iter()
                .find(|l| l.id == LayerId::Arp)
                .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Arp>());

            let Some(arp) = arp else {
                continue;
            };
            if arp.oper != 2 {
                continue;
            }
            if arp.spa != server_ip {
                continue;
            }

            dst_mac = Some(arp.sha);
            break;
        }

        if dst_mac.is_some() {
            break;
        }
    }

    let dst_mac = dst_mac.ok_or_else(|| anyhow::anyhow!("ARP resolve failed for 10.0.0.2"))?;
    eprintln!(
        "arp ok: {} is {}",
        ntx::network::fmt_ipv4!(server_ip),
        ntx::network::fmt_mac!(dst_mac),
    );

    // --- 2) Send UDP echo request (LayerInstance build) ---
    let app_payload = b"hello-echo";

    let frame = layers::Ether {
        dst: dst_mac,
        src: src_mac,
        ethertype: ntx::network::ETH_TYPE_IPV4,
    }
    .pkt()
    .ipv4(layers::Ipv4 {
        src: client_ip,
        dst: server_ip,
        proto: 17,
        ttl: 64,
        identification: 0,
        flags_fragment: 0,
        ihl_bytes: 20,
    })
    .udp(layers::Udp {
        src_port: 40000,
        dst_port: port,
        src_ip: None,
        dst_ip: None,
    })
    .raw(Raw::new(app_payload))
    .build(&reg)
    .map_err(anyhow::Error::msg)
    .context("build ether/ipv4/udp with payload")?;

    nic.send(&frame).context("send udp echo request")?;
    eprintln!("sent udp echo request: len={}", frame.len());

    // --- 3) Wait for reply ---
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
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
            Err(_) => continue,
        };
        // Filter: need Ether+Ipv4+Udp, and match server+ports. (dst mac + dst ip/port
        // were already filtered by accept() using ctx+ABR.)
        let ip = layers
            .iter()
            .find(|l| l.id == LayerId::Ipv4)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Ipv4>());
        let udp = layers
            .iter()
            .find(|l| l.id == LayerId::Udp)
            .and_then(|l| l.downcast_ref::<ntx::network::stack::layers::Udp>());

        let (Some(ip), Some(udp)) = (ip, udp) else {
            continue;
        };
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
