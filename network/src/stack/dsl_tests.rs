use crate::packet::headers::{Ipv4Addr, MacAddr};
use crate::stack::{LayerPkt, PacketBuilder, Raw, default_registry, layers};

#[test]
fn dsl_builds_ether_ipv4_tcp_raw() {
    let reg = default_registry();

    let pkt: PacketBuilder = layers::Ether {
        dst: MacAddr([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
        src: MacAddr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
        ethertype: crate::ETH_TYPE_IPV4,
    }
    .pkt()
    .ipv4(layers::Ipv4 {
        src: Ipv4Addr([10, 0, 0, 1]),
        dst: Ipv4Addr([10, 0, 0, 2]),
        proto: 6,
        ttl: 64,
        identification: 0,
        flags_fragment: 0,
        ihl_bytes: 20,
    })
    .tcp(layers::Tcp {
        src_port: 12345,
        dst_port: 80,
        data_offset_bytes: 20,
    })
    .raw(Raw::new(b"hello"));

    assert_eq!(pkt.layers.len(), 3);
    assert_eq!(pkt.payload.as_deref(), Some(b"hello".as_slice()));

    // TCP encode is minimal (it just forwards payload), but build should still succeed.
    let bytes = pkt.build(&reg).expect("build");
    assert!(bytes.len() >= 14 + 20 + 5);
}

#[test]
fn dsl_sets_payload_via_slice() {
    let reg = default_registry();
    let payload = b"abc";

    let pkt = layers::Ipv4 {
        src: Ipv4Addr([10, 0, 0, 1]),
        dst: Ipv4Addr([10, 0, 0, 2]),
        proto: 17,
        ttl: 64,
        identification: 0,
        flags_fragment: 0,
        ihl_bytes: 20,
    }
    .pkt()
    .udp(layers::Udp {
        src_port: 1,
        dst_port: 2,
        src_ip: None,
        dst_ip: None,
    })
    .payload(payload.as_slice());

    let bytes = pkt.build(&reg).expect("build");
    assert!(bytes.len() >= 20 + payload.len());
}
