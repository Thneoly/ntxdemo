use crate::guestnet::driver::{DriveReport, drive_once};
use crate::guestnet::flow::{EndpointV4, FlowManager, TransportProto};
use crate::guestnet::host_if::{Event, EventKind, PacketDesc, SharedMem};
use crate::guestnet::packet_io::{HostIf, PacketIo};
use crate::guestnet::socket_api::{SocketKind, SocketTable};

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

#[derive(Default)]
struct FakeHostIf {
    packets: std::collections::VecDeque<PacketDesc>,
}

impl FakeHostIf {
    fn push(&mut self, d: PacketDesc) {
        self.packets.push_back(d);
    }
}

impl HostIf for FakeHostIf {
    fn poll_packet(&mut self) -> Option<PacketDesc> {
        self.packets.pop_front()
    }

    fn poll_oneoff(&mut self, _interests: &[EventKind]) -> Vec<Event> {
        Vec::new()
    }

    fn tx_submit(&mut self, _frame: &[u8]) -> Result<(), crate::guestnet::host_if::TxError> {
        Err(crate::guestnet::host_if::TxError::Unsupported)
    }

    fn tx_submit_l3_ipv4(
        &mut self,
        _packet: &[u8],
    ) -> Result<(), crate::guestnet::host_if::TxError> {
        Err(crate::guestnet::host_if::TxError::Unsupported)
    }
}

fn build_udp_frame(
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
fn injected_hostif_allows_full_chain_packetio_to_socket_recv() {
    let frame = build_udp_frame([10, 0, 0, 1], [10, 0, 0, 2], 1111, 2222, b"ping");
    let shm = TestShm { backing: frame };

    let mut host = FakeHostIf::default();
    host.push(PacketDesc {
        buf_offset: 0,
        len: shm.backing.len() as u32,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 1,
    });

    let mut pio = PacketIo::with_host(&shm, host);

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

    let mut stats = None;
    drive_once(&mut pio, &mut flows, &mut socks, |r| {
        if let DriveReport::Stats(s) = r {
            stats = Some(s);
        }
    })
    .unwrap();
    let stats = stats.unwrap();

    assert_eq!(stats.packets_rx, 1);
    assert_eq!(stats.packets_bad_desc, 0);

    let msg = socks.recv(s, 64).unwrap();
    assert_eq!(msg, b"ping");
}
