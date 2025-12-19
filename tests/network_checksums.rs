use ntx::network::{
    ETH_TYPE_IPV4, EthernetHeader, Ipv4Addr, Ipv4Header, MacAddr, UdpHeader, ipv4_header_checksum,
    udp_checksum,
};

const IP_PROTO_UDP: u8 = 17;

#[test]
fn ipv4_header_checksum_matches_self() {
    // Build a minimal IPv4 header and verify checksum makes the header sum to 0xffff.
    let hdr = Ipv4Header {
        src: Ipv4Addr([192, 168, 1, 10]),
        dst: Ipv4Addr([192, 168, 1, 20]),
        protocol: IP_PROTO_UDP,
        ttl: 64,
        identification: 0x1234,
        flags_fragment: 0,
    };

    let mut out = [0u8; Ipv4Header::MIN_LEN];
    hdr.encode(&mut out, /*payload_len*/ 8, /*dscp_ecn*/ 0)
        .unwrap();

    let csum = u16::from_be_bytes([out[10], out[11]]);
    assert_ne!(csum, 0);

    // Recompute checksum over the header should match the embedded value.
    assert_eq!(csum, ipv4_header_checksum(&out));
}

#[test]
fn udp_checksum_basic_nonzero() {
    let src = Ipv4Addr([10, 0, 0, 1]);
    let dst = Ipv4Addr([10, 0, 0, 2]);

    let payload = b"hello";
    let udp = UdpHeader {
        src_port: 12345,
        dst_port: 10001,
    };

    let mut buf = vec![0u8; UdpHeader::LEN + payload.len()];
    udp.encode(&mut buf, payload, src, dst).unwrap();

    let embedded = u16::from_be_bytes([buf[6], buf[7]]);
    assert_ne!(embedded, 0);

    // Verify the checksum helper produces the same value (taking checksum field as zero).
    let csum = udp_checksum(src, dst, &buf);
    assert_eq!(embedded, csum);
}

#[test]
fn synthetic_frame_pipeline_parse_roundtrip() {
    // Ethernet + IPv4 + UDP + payload -> parse back and validate payload.
    let payload = b"ping";

    let src_mac = MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
    let dst_mac = MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]);

    let eth = EthernetHeader {
        dst: dst_mac,
        src: src_mac,
        ethertype: ETH_TYPE_IPV4,
    };

    let ip = Ipv4Header {
        src: Ipv4Addr([10, 1, 1, 1]),
        dst: Ipv4Addr([10, 1, 1, 2]),
        protocol: IP_PROTO_UDP,
        ttl: 64,
        identification: 0,
        flags_fragment: 0,
    };

    let udp = UdpHeader {
        src_port: 1234,
        dst_port: 10001,
    };

    let mut frame =
        vec![0u8; EthernetHeader::LEN + Ipv4Header::MIN_LEN + UdpHeader::LEN + payload.len()];

    // Write headers and payload
    eth.encode(&mut frame[..EthernetHeader::LEN]).unwrap();

    let ip_off = EthernetHeader::LEN;
    ip.encode(
        &mut frame[ip_off..ip_off + Ipv4Header::MIN_LEN],
        UdpHeader::LEN + payload.len(),
        0,
    )
    .unwrap();

    let udp_off = ip_off + Ipv4Header::MIN_LEN;
    udp.encode(
        &mut frame[udp_off..udp_off + UdpHeader::LEN + payload.len()],
        payload,
        ip.src,
        ip.dst,
    )
    .unwrap();

    // Parse back
    let (eth2, l3) = EthernetHeader::decode(&frame).unwrap();
    assert_eq!(eth2.ethertype, ETH_TYPE_IPV4);

    let (ip2, l4) = Ipv4Header::decode(l3).unwrap();
    assert_eq!(ip2.protocol, IP_PROTO_UDP);

    let (udp2, pl) = UdpHeader::decode(l4).unwrap();
    assert_eq!(udp2.dst_port, 10001);
    assert_eq!(pl, payload);
}
