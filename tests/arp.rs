use std::time::Duration;

use ntx::network::arp::{
    ARP_OP_REPLY, ARP_OP_REQUEST, ArpCache, ArpPacket, MAC_BROADCAST, build_arp_request_frame,
    parse_arp_reply,
};
use ntx::network::{ETH_TYPE_ARP, EthernetHeader, Ipv4Addr, MacAddr};

#[test]
fn arp_packet_write_parse_roundtrip_request() {
    let pkt = ArpPacket {
        oper: ARP_OP_REQUEST,
        sha: MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]),
        spa: Ipv4Addr([192, 168, 1, 10]),
        tha: MacAddr([0, 0, 0, 0, 0, 0]),
        tpa: Ipv4Addr([192, 168, 1, 1]),
    };

    let mut buf = [0u8; ArpPacket::LEN];
    pkt.encode(&mut buf).unwrap();

    let parsed = ArpPacket::decode(&buf).unwrap();
    assert_eq!(parsed, pkt);
}

#[test]
fn build_arp_request_frame_has_expected_ethernet_header() {
    let src_mac = MacAddr([0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
    let src_ip = Ipv4Addr([10, 0, 0, 2]);
    let target_ip = Ipv4Addr([10, 0, 0, 1]);

    let frame = build_arp_request_frame(src_mac, src_ip, target_ip).unwrap();

    let (eth, payload) = EthernetHeader::decode(&frame).unwrap();
    assert_eq!(eth.ethertype, ETH_TYPE_ARP);
    assert_eq!(eth.src, src_mac);
    assert_eq!(eth.dst, MAC_BROADCAST);

    let arp = ArpPacket::decode(payload).unwrap();
    assert_eq!(arp.oper, ARP_OP_REQUEST);
    assert_eq!(arp.sha, src_mac);
    assert_eq!(arp.spa, src_ip);
    assert_eq!(arp.tpa, target_ip);
}

#[test]
fn parse_arp_reply_extracts_sender() {
    let sender_mac = MacAddr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
    let sender_ip = Ipv4Addr([10, 0, 0, 1]);
    let target_mac = MacAddr([0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
    let target_ip = Ipv4Addr([10, 0, 0, 2]);

    // Build an Ethernet + ARP reply frame.
    let eth = EthernetHeader {
        dst: target_mac,
        src: sender_mac,
        ethertype: ETH_TYPE_ARP,
    };
    let arp = ArpPacket {
        oper: ARP_OP_REPLY,
        sha: sender_mac,
        spa: sender_ip,
        tha: target_mac,
        tpa: target_ip,
    };

    let mut frame = vec![0u8; EthernetHeader::LEN + ArpPacket::LEN];
    eth.encode(&mut frame[..EthernetHeader::LEN]).unwrap();
    arp.encode(&mut frame[EthernetHeader::LEN..]).unwrap();

    let got = parse_arp_reply(&frame).unwrap();
    assert_eq!(got, Some((sender_ip, sender_mac)));
}

#[test]
fn arp_cache_ttl_expires() {
    let mut cache = ArpCache::new(Duration::from_millis(1));
    let ip = Ipv4Addr([1, 2, 3, 4]);
    let mac = MacAddr([1, 2, 3, 4, 5, 6]);

    cache.insert(ip, mac);
    assert_eq!(cache.get(ip), Some(mac));

    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(cache.get(ip), None);
}
