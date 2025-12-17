use crate::stack::layers::register_all;
use crate::stack::{
    EdgeKind, LayerId, LayerInstance, LayerRegistry, build_packet_no_payload, parse_packet,
    parse_packet_graph,
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
