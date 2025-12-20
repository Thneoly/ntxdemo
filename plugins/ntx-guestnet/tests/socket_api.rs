use ntx_guestnet::flow::{EndpointV4, FlowManager, TransportProto};
use ntx_guestnet::host_if::{PacketDesc, SharedMem, packet_view_from_desc};
use ntx_guestnet::socket_api::{SocketError, SocketKind, SocketTable};

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
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    // Ethernet
    let mut out = vec![
        0u8;
        ntx_network::packet::headers::EthernetHeader::LEN
            + ntx_network::packet::headers::Ipv4Header::MIN_LEN
            + ntx_network::packet::headers::UdpHeader::LEN
            + payload.len()
    ];

    let eth = ntx_network::packet::headers::EthernetHeader {
        dst: ntx_network::packet::headers::MacAddr([6, 7, 8, 9, 10, 11]),
        src: ntx_network::packet::headers::MacAddr([0, 1, 2, 3, 4, 5]),
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
fn udp_socket_bind_receives_datagram() {
    let mut flows = FlowManager::new();
    flows.set_now_tick(1);

    let mut socks = SocketTable::new();
    let s = socks
        .socket(SocketKind::Datagram, TransportProto::Udp)
        .unwrap();
    socks
        .bind(
            &mut flows,
            s,
            EndpointV4 {
                ip: [10, 0, 0, 2],
                port: 2222,
            },
        )
        .unwrap();

    // Inbound packet to 10.0.0.2:2222
    let frame = build_udp_frame([10, 0, 0, 1], [10, 0, 0, 2], 1111, 2222, b"ping");
    let shm = TestShm { backing: frame };
    let desc = PacketDesc {
        buf_offset: 0,
        len: shm.backing.len() as u32,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 1,
    };
    let view = packet_view_from_desc(&shm, desc).unwrap();

    socks.on_packet(&mut flows, view).unwrap();
    socks
        .pump_transport_to_sockets_with_report(&mut flows)
        .unwrap();

    let msg = socks.recv(s, 64).unwrap();
    assert_eq!(msg, b"ping");
}

#[test]
fn udp_socket_recv_would_block_when_empty() {
    let mut flows = FlowManager::new();
    flows.set_now_tick(1);

    let mut socks = SocketTable::new();
    let s = socks
        .socket(SocketKind::Datagram, TransportProto::Udp)
        .unwrap();

    assert!(matches!(socks.recv(s, 64), Err(SocketError::WouldBlock)));
}

#[test]
fn udp_socket_send_requires_connect() {
    let mut flows = FlowManager::new();
    flows.set_now_tick(1);

    let mut socks = SocketTable::new();
    let s = socks
        .socket(SocketKind::Datagram, TransportProto::Udp)
        .unwrap();

    socks
        .bind(
            &mut flows,
            s,
            EndpointV4 {
                ip: [10, 0, 0, 2],
                port: 2222,
            },
        )
        .unwrap();

    // Not connected yet.
    assert!(matches!(
        socks.send(s, b"hello"),
        Err(SocketError::InvalidState)
    ));

    socks
        .connect(
            &mut flows,
            s,
            EndpointV4 {
                ip: [10, 0, 0, 1],
                port: 1111,
            },
        )
        .unwrap();

    // Now send is accepted into transport tx queue.
    let n = socks.send(s, b"hello").unwrap();
    assert_eq!(n, 5);
}

#[test]
fn udp_socket_on_packet_malformed_is_structured() {
    let mut flows = FlowManager::new();
    flows.set_now_tick(1);

    let mut socks = SocketTable::new();

    let shm = TestShm {
        backing: vec![0u8; 8],
    };
    let desc = PacketDesc {
        buf_offset: 0,
        len: shm.backing.len() as u32,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 1,
    };
    let view = packet_view_from_desc(&shm, desc).unwrap();

    let e = socks.on_packet(&mut flows, view).unwrap_err();
    let s = e.to_string();
    assert!(s.contains("malformed packet"), "unexpected error: {s}");
}
