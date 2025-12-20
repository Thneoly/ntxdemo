use ntx_guestnet::flow::{EndpointV4, FlowManager, SocketBindKey, SocketId, TransportProto};
use ntx_guestnet::host_if::{PacketDesc, SharedMem, packet_view_from_desc};
use ntx_guestnet::transport::{Transport, TransportError, UdpTransport};

struct TestShm {
    backing: Vec<u8>,
}

impl SharedMem for TestShm {
    fn get_range(&self, range: core::ops::Range<u32>) -> Option<&[u8]> {
        let start = usize::try_from(range.start).ok()?;
        let end = usize::try_from(range.end).ok()?;
        self.backing.get(start..end)
    }
}

fn build_udp_frame(
    src_mac: [u8; 6],
    dst_mac: [u8; 6],
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = vec![
        0u8;
        ntx_network::packet::headers::EthernetHeader::LEN
            + ntx_network::packet::headers::Ipv4Header::MIN_LEN
            + ntx_network::packet::headers::UdpHeader::LEN
            + payload.len()
    ];

    let eth = ntx_network::packet::headers::EthernetHeader {
        dst: ntx_network::packet::headers::MacAddr(dst_mac),
        src: ntx_network::packet::headers::MacAddr(src_mac),
        ethertype: ntx_network::packet::headers::ETH_TYPE_IPV4,
    };

    eth.encode(&mut out[..ntx_network::packet::headers::EthernetHeader::LEN])
        .unwrap();

    let ip = ntx_network::packet::headers::Ipv4Header {
        src: ntx_network::packet::headers::Ipv4Addr(src_ip),
        dst: ntx_network::packet::headers::Ipv4Addr(dst_ip),
        protocol: 17,
        ttl: 64,
        identification: 0,
        flags_fragment: 0,
    };

    let ip_off = ntx_network::packet::headers::EthernetHeader::LEN;
    ip.encode(
        &mut out[ip_off..ip_off + ntx_network::packet::headers::Ipv4Header::MIN_LEN],
        ntx_network::packet::headers::UdpHeader::LEN + payload.len(),
        0,
    )
    .unwrap();

    let udp = ntx_network::packet::headers::UdpHeader { src_port, dst_port };
    let udp_off = ip_off + ntx_network::packet::headers::Ipv4Header::MIN_LEN;
    udp.encode(
        &mut out[udp_off..udp_off + ntx_network::packet::headers::UdpHeader::LEN + payload.len()],
        payload,
        ntx_network::packet::headers::Ipv4Addr(src_ip),
        ntx_network::packet::headers::Ipv4Addr(dst_ip),
    )
    .unwrap();

    out
}

#[test]
fn udp_on_packet_delivers_to_bound_socket_queue() {
    let frame = build_udp_frame(
        [0, 1, 2, 3, 4, 5],
        [6, 7, 8, 9, 10, 11],
        [10, 0, 0, 1],
        [10, 0, 0, 2],
        1111,
        2222,
        b"abcd",
    );

    let shm = TestShm { backing: frame };
    let desc = PacketDesc {
        buf_offset: 0,
        len: shm.backing.len() as u32,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 1,
    };
    let view = packet_view_from_desc(&shm, desc).unwrap();

    let mut flows = FlowManager::new();
    flows.set_now_tick(1);

    flows.bind_socket(
        SocketBindKey {
            proto: TransportProto::Udp,
            local: EndpointV4 {
                ip: [10, 0, 0, 2],
                port: 2222,
            },
            remote: None,
        },
        SocketId(9),
    );

    let mut udp = UdpTransport::new(8);
    udp.on_packet(&mut flows, view).unwrap();

    let dg = udp.poll_recv(SocketId(9)).unwrap();
    assert_eq!(dg.payload, b"abcd");
    assert_eq!(dg.src, ([10, 0, 0, 1], 1111));
    assert_eq!(dg.dst, ([10, 0, 0, 2], 2222));

    assert!(matches!(
        udp.poll_recv(SocketId(9)),
        Err(TransportError::WouldBlock)
    ));
}

#[test]
fn udp_on_packet_drops_when_unbound() {
    let frame = build_udp_frame(
        [0, 1, 2, 3, 4, 5],
        [6, 7, 8, 9, 10, 11],
        [10, 0, 0, 1],
        [10, 0, 0, 2],
        1111,
        2222,
        b"abcd",
    );

    let shm = TestShm { backing: frame };
    let desc = PacketDesc {
        buf_offset: 0,
        len: shm.backing.len() as u32,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 1,
    };
    let view = packet_view_from_desc(&shm, desc).unwrap();

    let mut flows = FlowManager::new();
    flows.set_now_tick(1);

    let mut udp = UdpTransport::new(8);
    udp.on_packet(&mut flows, view).unwrap();

    assert!(matches!(
        udp.poll_recv(SocketId(999)),
        Err(TransportError::WouldBlock)
    ));
}

#[test]
fn udp_backpressure_returns_would_block() {
    let frame = build_udp_frame(
        [0, 1, 2, 3, 4, 5],
        [6, 7, 8, 9, 10, 11],
        [10, 0, 0, 1],
        [10, 0, 0, 2],
        1111,
        2222,
        b"abcd",
    );

    let shm = TestShm { backing: frame };
    let desc = PacketDesc {
        buf_offset: 0,
        len: shm.backing.len() as u32,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 1,
    };
    let view = packet_view_from_desc(&shm, desc).unwrap();

    let mut flows = FlowManager::new();
    flows.set_now_tick(1);

    flows.bind_socket(
        SocketBindKey {
            proto: TransportProto::Udp,
            local: EndpointV4 {
                ip: [10, 0, 0, 2],
                port: 2222,
            },
            remote: None,
        },
        SocketId(9),
    );

    let mut udp = UdpTransport::new(0);
    let r = udp.on_packet(&mut flows, view);
    assert!(matches!(r, Err(TransportError::WouldBlock)));
}

#[test]
fn udp_on_packet_primes_conntable_and_can_build_reply() {
    // Inbound frame: client -> server
    let frame = build_udp_frame(
        [0, 1, 2, 3, 4, 5],
        [6, 7, 8, 9, 10, 11],
        [10, 0, 0, 1],
        [10, 0, 0, 2],
        1111,
        2222,
        b"ping",
    );

    let shm = TestShm { backing: frame };
    let desc = PacketDesc {
        buf_offset: 0,
        len: shm.backing.len() as u32,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 1,
    };
    let view = packet_view_from_desc(&shm, desc).unwrap();

    let mut flows = FlowManager::new();
    flows.set_now_tick(1);
    flows.bind_socket(
        SocketBindKey {
            proto: TransportProto::Udp,
            local: EndpointV4 {
                ip: [10, 0, 0, 2],
                port: 2222,
            },
            remote: None,
        },
        SocketId(9),
    );

    let mut udp = UdpTransport::new(8);
    udp.on_packet(&mut flows, view).unwrap();

    // Now the transport should have learned a reverse-path reply template.
    // Build a reply: server -> client
    let key = ntx_network::stack::UdpFlowKey {
        peer_ip: ntx_network::packet::headers::Ipv4Addr([10, 0, 0, 1]),
        peer_port: 1111,
        local_ip: ntx_network::packet::headers::Ipv4Addr([10, 0, 0, 2]),
        local_port: 2222,
    };
    let conn = udp
        .conns_mut()
        .get(&key)
        .expect("ConnTable should be primed by on_packet()");
    let reply = conn.build_reply(b"pong").unwrap();

    let (eth, l3) = ntx_network::packet::headers::EthernetHeader::decode(&reply.bytes).unwrap();
    let (ip, l4) = ntx_network::packet::headers::Ipv4Header::decode(l3).unwrap();
    let (udp_hdr, payload) = ntx_network::packet::headers::UdpHeader::decode(l4).unwrap();

    // Reply should go back along the same L2 path: dst=original src, src=original dst.
    assert_eq!(eth.src.0, [6, 7, 8, 9, 10, 11]);
    assert_eq!(eth.dst.0, [0, 1, 2, 3, 4, 5]);

    assert_eq!(ip.src.0, [10, 0, 0, 2]);
    assert_eq!(ip.dst.0, [10, 0, 0, 1]);
    assert_eq!(udp_hdr.src_port, 2222);
    assert_eq!(udp_hdr.dst_port, 1111);
    assert_eq!(payload, b"pong");
}
