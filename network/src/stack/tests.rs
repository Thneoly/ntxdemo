use crate::abr::{Binding, BindingOwner, BindingStore};
use crate::stack::layers::register_all;
use crate::stack::{
    EdgeKind, LayerId, LayerInstance, LayerRegistry, PacketContext, build_packet_no_payload,
    build_packet_with_glue, parse_packet, parse_packet_graph, parse_packet_with_ctx,
};

#[test]
fn decode_chain_eth_ipv4_udp() {
    // Build a minimal Ethernet + IPv4 + UDP frame.
    // Ethernet: dst/src + ethertype
    let mut frame = Vec::new();
    frame.extend_from_slice(&[0u8; 6]); // dst
    frame.extend_from_slice(&[1u8; 6]); // src
    frame.extend_from_slice(&0x0800u16.to_be_bytes()); // IPv4

    // IPv4 header (20 bytes)
    // version+ihl
    frame.push(0x45);
    // dscp+ecn
    frame.push(0);
    // total length = 20 + 8 + 4
    frame.extend_from_slice(&(32u16).to_be_bytes());
    // identification
    frame.extend_from_slice(&0u16.to_be_bytes());
    // flags+frag
    frame.extend_from_slice(&0u16.to_be_bytes());
    // ttl
    frame.push(64);
    // proto=UDP
    frame.push(17);
    // header checksum placeholder
    frame.extend_from_slice(&0u16.to_be_bytes());
    // src ip 10.0.0.1
    frame.extend_from_slice(&[10, 0, 0, 1]);
    // dst ip 10.0.0.2
    frame.extend_from_slice(&[10, 0, 0, 2]);

    // fill ipv4 checksum
    let csum = crate::ipv4_header_checksum(&frame[14..34]);
    frame[24] = (csum >> 8) as u8;
    frame[25] = (csum & 0xff) as u8;

    // UDP header (8 bytes)
    frame.extend_from_slice(&1234u16.to_be_bytes());
    frame.extend_from_slice(&4321u16.to_be_bytes());
    frame.extend_from_slice(&(12u16).to_be_bytes()); // len=8+4
    frame.extend_from_slice(&0u16.to_be_bytes()); // checksum placeholder

    // payload
    frame.extend_from_slice(b"ping");

    // Fill UDP checksum
    let udp_off = 14 + 20;
    let src = crate::Ipv4Addr([10, 0, 0, 1]);
    let dst = crate::Ipv4Addr([10, 0, 0, 2]);
    let csum = crate::udp_checksum(src, dst, &frame[udp_off..udp_off + 12]);
    frame[udp_off + 6] = (csum >> 8) as u8;
    frame[udp_off + 7] = (csum & 0xff) as u8;

    let mut reg = LayerRegistry::new();
    register_all(&mut reg);
    let (layers, payload) = parse_packet(&frame, LayerId::Ether, &reg).unwrap();

    assert!(layers.iter().any(|l| l.id == LayerId::Ether));
    assert!(layers.iter().any(|l| l.id == LayerId::Ipv4));
    assert!(layers.iter().any(|l| l.id == LayerId::Udp));
    assert_eq!(payload, b"ping");
}

#[test]
fn decode_graph_udp_vxlan_inner_ether() {
    // Outer Ethernet + IPv4 + UDP(dport=4789)
    let mut frame = Vec::new();
    frame.extend_from_slice(&[0u8; 6]); // dst
    frame.extend_from_slice(&[1u8; 6]); // src
    frame.extend_from_slice(&0x0800u16.to_be_bytes()); // IPv4

    // IPv4 header (20 bytes)
    frame.push(0x45);
    frame.push(0);
    // total length filled later
    frame.extend_from_slice(&0u16.to_be_bytes());
    frame.extend_from_slice(&0u16.to_be_bytes()); // identification
    frame.extend_from_slice(&0u16.to_be_bytes()); // flags+frag
    frame.push(64);
    frame.push(17); // UDP
    frame.extend_from_slice(&0u16.to_be_bytes()); // hdr checksum placeholder
    frame.extend_from_slice(&[10, 0, 0, 1]);
    frame.extend_from_slice(&[10, 0, 0, 2]);

    // UDP header (8 bytes) placeholder
    frame.extend_from_slice(&1234u16.to_be_bytes());
    frame.extend_from_slice(&4789u16.to_be_bytes());
    frame.extend_from_slice(&0u16.to_be_bytes()); // len placeholder
    frame.extend_from_slice(&0u16.to_be_bytes()); // checksum placeholder

    // VXLAN header (8 bytes)
    frame.push(0x08); // I flag
    frame.extend_from_slice(&[0, 0, 0]);
    // vni=1
    frame.extend_from_slice(&[0, 0, 1]);
    frame.push(0);

    // Inner Ethernet header + payload "hi"
    frame.extend_from_slice(&[2u8; 6]);
    frame.extend_from_slice(&[3u8; 6]);
    // Use a non-IPv4 ethertype so the inner parse stops at Ether and leaves payload as bytes.
    frame.extend_from_slice(&0x88B5u16.to_be_bytes());
    frame.extend_from_slice(b"hi");

    // Fill lengths + checksums
    let eth_len = 14usize;
    let ip_len = 20usize;
    let udp_len = 8usize;
    let udp_payload_len = 8usize /*vxlan*/ + 14usize /*inner eth*/ + 2usize;
    let total_len = (ip_len + udp_len + udp_payload_len) as u16;
    frame[eth_len + 2] = (total_len >> 8) as u8;
    frame[eth_len + 3] = (total_len & 0xff) as u8;

    let ip_csum = crate::ipv4_header_checksum(&frame[eth_len..eth_len + ip_len]);
    frame[eth_len + 10] = (ip_csum >> 8) as u8;
    frame[eth_len + 11] = (ip_csum & 0xff) as u8;

    let udp_off = eth_len + ip_len;
    let udp_total_len = (udp_len + udp_payload_len) as u16;
    frame[udp_off + 4] = (udp_total_len >> 8) as u8;
    frame[udp_off + 5] = (udp_total_len & 0xff) as u8;

    let src = crate::Ipv4Addr([10, 0, 0, 1]);
    let dst = crate::Ipv4Addr([10, 0, 0, 2]);
    let udp_csum = crate::udp_checksum(
        src,
        dst,
        &frame[udp_off..udp_off + udp_len + udp_payload_len],
    );
    frame[udp_off + 6] = (udp_csum >> 8) as u8;
    frame[udp_off + 7] = (udp_csum & 0xff) as u8;

    // Sanity: parse chain sees UDP and stops at VXLAN due to binding.
    let mut reg = LayerRegistry::new();
    register_all(&mut reg);
    let (layers, _payload) = parse_packet(&frame, LayerId::Ether, &reg).unwrap();
    assert!(layers.iter().any(|l| l.id == LayerId::Udp));
    assert!(layers.iter().any(|l| l.id == LayerId::Vxlan));

    // Graph should include an inner Ether node linked by a tunnel edge.
    let g = parse_packet_graph(&frame, LayerId::Ether, &reg).unwrap();

    let vx_idx = g
        .nodes()
        .iter()
        .position(|n| n.id == LayerId::Vxlan)
        .expect("missing vxlan node");

    // Find the tunneled-to node via the tunnel edge itself.
    let inner_idx = g
        .edges()
        .iter()
        .find(|(a, _b, k)| *a == vx_idx && *k == EdgeKind::Tunnels)
        .map(|(_a, b, _k)| *b)
        .expect("missing tunnel edge from vxlan");

    assert_eq!(g.nodes()[inner_idx].id, LayerId::Ether);
}

#[test]
fn build_no_payload_eth_ipv4_udp_roundtrip() {
    // Build Ether/IPv4/UDP with no payload and ensure we can parse it back.
    // This validates the ergonomic builder wrapper while keeping registry explicit.
    let mut reg = LayerRegistry::new();
    register_all(&mut reg);

    let layers = vec![
        LayerInstance {
            id: LayerId::Ether,
            inner: Box::new(crate::stack::layers::Ether {
                src: crate::MacAddr([1, 1, 1, 1, 1, 1]),
                dst: crate::MacAddr([2, 2, 2, 2, 2, 2]),
                ethertype: crate::ETH_TYPE_IPV4,
            }),
        },
        LayerInstance {
            id: LayerId::Ipv4,
            inner: Box::new(crate::stack::layers::Ipv4 {
                src: crate::Ipv4Addr([10, 0, 0, 1]),
                dst: crate::Ipv4Addr([10, 0, 0, 2]),
                ttl: 64,
                proto: 17,
                identification: 0,
                flags_fragment: 0,
                ihl_bytes: 20,
            }),
        },
        LayerInstance {
            id: LayerId::Udp,
            inner: Box::new(crate::stack::layers::Udp {
                src_port: 1234,
                dst_port: 4321,
                src_ip: None,
                dst_ip: None,
            }),
        },
    ];

    let bytes = build_packet_no_payload(&layers, &reg).unwrap();
    let (parsed, payload) = parse_packet(&bytes, LayerId::Ether, &reg).unwrap();

    assert!(parsed.iter().any(|l| l.id == LayerId::Ether));
    assert!(parsed.iter().any(|l| l.id == LayerId::Ipv4));
    assert!(parsed.iter().any(|l| l.id == LayerId::Udp));
    assert_eq!(payload, b"");
}

#[test]
fn accept_ipv4_not_owned_poison_stops_before_udp() {
    let mut reg = LayerRegistry::new();
    register_all(&mut reg);

    let layers = vec![
        LayerInstance {
            id: LayerId::Ether,
            inner: Box::new(crate::stack::layers::Ether {
                src: crate::MacAddr([1, 1, 1, 1, 1, 1]),
                dst: crate::MacAddr([2, 2, 2, 2, 2, 2]),
                ethertype: crate::ETH_TYPE_IPV4,
            }),
        },
        LayerInstance {
            id: LayerId::Ipv4,
            inner: Box::new(crate::stack::layers::Ipv4 {
                src: crate::Ipv4Addr([10, 0, 0, 1]),
                dst: crate::Ipv4Addr([10, 0, 0, 123]),
                ttl: 64,
                proto: 17,
                identification: 0,
                flags_fragment: 0,
                ihl_bytes: 20,
            }),
        },
        LayerInstance {
            id: LayerId::Udp,
            inner: Box::new(crate::stack::layers::Udp {
                src_port: 1234,
                dst_port: 7,
                src_ip: None,
                dst_ip: None,
            }),
        },
    ];

    let frame = build_packet_with_glue(&layers, b"hi", &reg).unwrap();
    let mut store = BindingStore::default();
    store.add(Binding::ipv4_be(0x0a00_0001, BindingOwner::KernelIface)); // 10.0.0.1
    let view = store.snapshot();

    let ctx = PacketContext {
        iface_mac: None,
        abr: Some(std::sync::Arc::new(view)),
        local_ipv4: vec![],
    };

    let (decoded, payload) = parse_packet_with_ctx(&frame, LayerId::Ether, &reg, &ctx).unwrap();
    assert!(decoded.iter().any(|l| l.id == LayerId::Ether));
    assert!(decoded.iter().any(|l| l.id == LayerId::Ipv4));
    assert!(!decoded.iter().any(|l| l.id == LayerId::Udp));

    // Because we stop at IPv4, the remaining bytes are the UDP header + payload.
    assert_eq!(payload.len(), 8 + 2);
}

#[test]
fn accept_ether_not_to_us_drop_errors() {
    let mut reg = LayerRegistry::new();
    register_all(&mut reg);

    // Minimal ether header.
    let mut frame = Vec::new();
    frame.extend_from_slice(&[0x10, 0x10, 0x10, 0x10, 0x10, 0x10]); // dst
    frame.extend_from_slice(&[0x20, 0x20, 0x20, 0x20, 0x20, 0x20]); // src
    frame.extend_from_slice(&0x0800u16.to_be_bytes()); // IPv4
    frame.extend_from_slice(&[0u8; 20]);

    let ctx = PacketContext {
        iface_mac: Some(crate::MacAddr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff])),
        abr: None,
        local_ipv4: vec![],
    };

    let err = parse_packet_with_ctx(&frame, LayerId::Ether, &reg, &ctx).unwrap_err();
    assert_eq!(
        err.to_string(),
        "dropped by accept(): layer=Ether result=Drop".to_string()
    );
}

#[test]
fn accept_arp_tpa_not_owned_poison_stops_at_arp() {
    let mut reg = LayerRegistry::new();
    register_all(&mut reg);

    // Ethernet + ARP request for 10.0.0.2
    let eth = LayerInstance {
        id: LayerId::Ether,
        inner: Box::new(crate::stack::layers::Ether {
            src: crate::MacAddr([1, 1, 1, 1, 1, 1]),
            dst: crate::MacAddr([0xff; 6]),
            ethertype: crate::ETH_TYPE_ARP,
        }),
    };
    let arp = LayerInstance {
        id: LayerId::Arp,
        inner: Box::new(crate::stack::layers::Arp {
            oper: 1,
            sha: crate::MacAddr([1, 1, 1, 1, 1, 1]),
            spa: crate::Ipv4Addr([10, 0, 0, 1]),
            tha: crate::MacAddr([0, 0, 0, 0, 0, 0]),
            tpa: crate::Ipv4Addr([10, 0, 0, 2]),
        }),
    };

    let frame = build_packet_no_payload(&[eth, arp], &reg).unwrap();

    // ABR owns only 10.0.0.1, not 10.0.0.2
    let mut store = BindingStore::default();
    store.add(Binding::ipv4_be(0x0a00_0001, BindingOwner::KernelIface));
    let ctx = PacketContext {
        iface_mac: None,
        abr: Some(std::sync::Arc::new(store.snapshot())),
        local_ipv4: vec![],
    };

    let (layers, _payload) = parse_packet_with_ctx(&frame, LayerId::Ether, &reg, &ctx).unwrap();
    assert!(layers.iter().any(|l| l.id == LayerId::Ether));
    assert!(layers.iter().any(|l| l.id == LayerId::Arp));
    // Stop at ARP, nothing above.
    assert_eq!(layers.last().unwrap().id, LayerId::Arp);
}

#[test]
fn accept_udp_port_not_bound_poison_stops_before_udp() {
    let mut reg = LayerRegistry::new();
    register_all(&mut reg);

    let layers = vec![
        LayerInstance {
            id: LayerId::Ether,
            inner: Box::new(crate::stack::layers::Ether {
                src: crate::MacAddr([1, 1, 1, 1, 1, 1]),
                dst: crate::MacAddr([2, 2, 2, 2, 2, 2]),
                ethertype: crate::ETH_TYPE_IPV4,
            }),
        },
        LayerInstance {
            id: LayerId::Ipv4,
            inner: Box::new(crate::stack::layers::Ipv4 {
                src: crate::Ipv4Addr([10, 0, 0, 1]),
                dst: crate::Ipv4Addr([10, 0, 0, 2]),
                ttl: 64,
                proto: 17,
                identification: 0,
                flags_fragment: 0,
                ihl_bytes: 20,
            }),
        },
        LayerInstance {
            id: LayerId::Udp,
            inner: Box::new(crate::stack::layers::Udp {
                src_port: 1234,
                dst_port: 9999,
                src_ip: None,
                dst_ip: None,
            }),
        },
    ];

    let frame = build_packet_with_glue(&layers, b"hi", &reg).unwrap();

    let mut store = BindingStore::default();
    // Own dst ip so IPv4 accept passes, but do NOT bind udp port 9999.
    store.add(Binding::ipv4_be(0x0a00_0002, BindingOwner::KernelIface));
    // Bind some other UDP port (wildcard ip).
    store.add(Binding::udp_port_be(0, 7, BindingOwner::KernelIface));

    let ctx = PacketContext {
        iface_mac: None,
        abr: Some(std::sync::Arc::new(store.snapshot())),
        local_ipv4: vec![],
    };

    let (decoded, payload) = parse_packet_with_ctx(&frame, LayerId::Ether, &reg, &ctx).unwrap();
    assert!(decoded.iter().any(|l| l.id == LayerId::Ether));
    assert!(decoded.iter().any(|l| l.id == LayerId::Ipv4));
    // UDP is decoded, then accept() returns Poison, so the chain stops *at* UDP.
    assert!(decoded.iter().any(|l| l.id == LayerId::Udp));
    assert_eq!(decoded.last().unwrap().id, LayerId::Udp);

    // Because we stop at UDP, the remaining bytes are the UDP payload ("hi").
    assert_eq!(payload, b"hi");
}

#[test]
fn accept_vxlan_vni_not_bound_poison_stops_at_vxlan() {
    let mut reg = LayerRegistry::new();
    register_all(&mut reg);

    // Outer Ether + IPv4 + UDP(dport=4789) + VXLAN(vni=42) + inner ether bytes.
    let mut frame = Vec::new();

    // Ether
    frame.extend_from_slice(&[0u8; 6]);
    frame.extend_from_slice(&[1u8; 6]);
    frame.extend_from_slice(&0x0800u16.to_be_bytes());

    // IPv4 header minimal (checksum left 0, but total_len must be valid)
    frame.push(0x45);
    frame.push(0);
    // total_len placeholder; fill later
    let total_len_off = frame.len();
    frame.extend_from_slice(&0u16.to_be_bytes());
    frame.extend_from_slice(&0u16.to_be_bytes());
    frame.extend_from_slice(&0u16.to_be_bytes());
    frame.push(64);
    frame.push(17);
    frame.extend_from_slice(&0u16.to_be_bytes());
    frame.extend_from_slice(&[10, 0, 0, 1]);
    frame.extend_from_slice(&[10, 0, 0, 2]);

    // UDP header (4789)
    frame.extend_from_slice(&1234u16.to_be_bytes());
    frame.extend_from_slice(&4789u16.to_be_bytes());
    // udp len placeholder
    let udp_len_off = frame.len();
    frame.extend_from_slice(&0u16.to_be_bytes());
    frame.extend_from_slice(&0u16.to_be_bytes());

    // VXLAN vni=42
    frame.push(0x08);
    frame.extend_from_slice(&[0, 0, 0]);
    frame.extend_from_slice(&[0, 0, 42]);
    frame.push(0);

    // inner eth header only (14 bytes)
    frame.extend_from_slice(&[2u8; 6]);
    frame.extend_from_slice(&[3u8; 6]);
    frame.extend_from_slice(&0x88B5u16.to_be_bytes());

    // Fill IPv4 total_len and UDP length so decoding succeeds.
    let ip_header_len = 20usize;
    let ip_payload_len = frame.len() - 14 - ip_header_len;
    let total_len = (ip_header_len + ip_payload_len) as u16;
    frame[total_len_off] = (total_len >> 8) as u8;
    frame[total_len_off + 1] = (total_len & 0xff) as u8;

    let udp_payload_len = frame.len() - (14 + ip_header_len + 8);
    let udp_len = (8 + udp_payload_len) as u16;
    frame[udp_len_off] = (udp_len >> 8) as u8;
    frame[udp_len_off + 1] = (udp_len & 0xff) as u8;

    let mut store = BindingStore::default();
    // Own IPv4 dst so we reach UDP/VXLAN.
    store.add(Binding::ipv4_be(0x0a00_0002, BindingOwner::KernelIface));
    // bind udp 4789 so we parse VXLAN
    store.add(Binding::udp_port_be(0, 4789, BindingOwner::KernelIface));
    // Configure a non-empty VNI set that does NOT include 42, so VXLAN accept() will Poison.
    store.add(Binding {
        kind: crate::abr::ResourceKind::Vni,
        key: crate::abr::BindingKey::Vni(1),
        owner: crate::abr::BindingOwner::Tunnel { id: 1 },
        flags: crate::abr::BindingFlags::NONE,
    });

    let ctx = PacketContext {
        iface_mac: None,
        abr: Some(std::sync::Arc::new(store.snapshot())),
        local_ipv4: vec![],
    };

    let (decoded, _payload) = parse_packet_with_ctx(&frame, LayerId::Ether, &reg, &ctx).unwrap();
    assert!(decoded.iter().any(|l| l.id == LayerId::Ipv4));
    assert!(decoded.iter().any(|l| l.id == LayerId::Udp));
    assert!(decoded.iter().any(|l| l.id == LayerId::Vxlan));

    // VXLAN accept() returns Poison because vni=42 isn't in ABR. The chain stops at VXLAN.
    assert_eq!(decoded.last().unwrap().id, LayerId::Vxlan);
}

#[test]
fn decode_chain_eth_arp() {
    // Build an Ethernet + ARP request and ensure it decodes into Ether->Arp.
    let eth = crate::EthernetHeader {
        dst: crate::MacAddr([0xff; 6]),
        src: crate::MacAddr([1, 2, 3, 4, 5, 6]),
        ethertype: crate::ETH_TYPE_ARP,
    };
    let arp = crate::ArpPacket {
        oper: 1,
        sha: eth.src,
        spa: crate::Ipv4Addr([10, 0, 0, 1]),
        tha: crate::MacAddr([0, 0, 0, 0, 0, 0]),
        tpa: crate::Ipv4Addr([10, 0, 0, 2]),
    };
    let mut frame = vec![0u8; crate::EthernetHeader::LEN + crate::ArpPacket::LEN];
    eth.encode(&mut frame[..crate::EthernetHeader::LEN])
        .unwrap();
    arp.encode(&mut frame[crate::EthernetHeader::LEN..])
        .unwrap();

    let mut reg = LayerRegistry::new();
    register_all(&mut reg);
    let (layers, payload) = parse_packet(&frame, LayerId::Ether, &reg).unwrap();

    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].id, LayerId::Ether);
    assert_eq!(layers[1].id, LayerId::Arp);
    assert_eq!(payload, b"");
}

#[test]
fn build_no_payload_eth_arp_roundtrip() {
    // Build Ether/ARP with no payload and ensure we can parse it back.
    let mut reg = LayerRegistry::new();
    register_all(&mut reg);

    let src_mac = crate::MacAddr([1, 2, 3, 4, 5, 6]);
    let dst_mac = crate::MacAddr([0xff; 6]);
    let src_ip = crate::Ipv4Addr([10, 0, 0, 1]);
    let dst_ip = crate::Ipv4Addr([10, 0, 0, 2]);

    let layers = vec![
        LayerInstance {
            id: LayerId::Ether,
            inner: Box::new(crate::stack::layers::Ether {
                dst: dst_mac,
                src: src_mac,
                ethertype: crate::ETH_TYPE_ARP,
            }),
        },
        LayerInstance {
            id: LayerId::Arp,
            inner: Box::new(crate::stack::layers::Arp {
                oper: 1,
                sha: src_mac,
                spa: src_ip,
                tha: crate::MacAddr([0, 0, 0, 0, 0, 0]),
                tpa: dst_ip,
            }),
        },
    ];

    let bytes = build_packet_no_payload(&layers, &reg).unwrap();
    let (parsed, payload) = parse_packet(&bytes, LayerId::Ether, &reg).unwrap();

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].id, LayerId::Ether);
    assert_eq!(parsed[1].id, LayerId::Arp);
    assert_eq!(payload, b"");
}
