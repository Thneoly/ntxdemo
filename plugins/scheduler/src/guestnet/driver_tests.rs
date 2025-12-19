use crate::guestnet::driver::{DriveReport, drive_once};
use crate::guestnet::flow::{EndpointV4, FlowManager, TransportProto};
use crate::guestnet::host_if::{PacketDesc, SharedMem, packet_view_from_desc};
use crate::guestnet::packet_io::PacketIo;
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
fn drive_once_pumps_transport_into_socket_rx() {
    // NOTE: PacketIo currently polls host_if::poll_packet() which is a stub returning None,
    // so drive_once won't ingest packets through PacketIo in unit tests (yet).
    // Instead we validate that the second stage (pump_transport_to_sockets) is called and works
    // by directly injecting a packet through sockets.on_packet(), then calling drive_once.

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

    // Inject one packet into transport.
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

    // Now drive_once should pump transport -> socket rx (PacketIo stage is a no-op in tests).
    let mut pio = PacketIo::new(&shm);
    let mut seen = None;
    drive_once(&mut pio, &mut flows, &mut socks, |r| {
        if let DriveReport::Stats(s) = r {
            seen = Some(s);
        }
    })
    .unwrap();
    let _stats = seen.expect("stats callback");

    let msg = socks.recv(s, 64).unwrap();
    assert_eq!(msg, b"ping");
}

#[test]
fn drive_once_reports_on_packet_error_reason() {
    // Malformed frame: too short to contain ethernet+ipv4+udp.
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

    let mut flows = FlowManager::new();
    flows.set_now_tick(1);
    let mut socks = SocketTable::new();

    // First, prove the packet actually errors at the socket ingestion boundary.
    assert!(socks.on_packet(&mut flows, view).is_err());

    // Now, feed the same malformed bytes through drive_once using the injected HostIf.
    // We only care that the error reason is surfaced via the reporting hook.
    use crate::guestnet::host_if::{Event, EventKind};
    use crate::guestnet::packet_io::HostIf;

    #[derive(Default)]
    struct OnePacketHostIf {
        p: Option<PacketDesc>,
    }

    impl HostIf for OnePacketHostIf {
        fn poll_packet(&mut self) -> Option<PacketDesc> {
            self.p.take()
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

    let mut pio = PacketIo::with_host(&shm, OnePacketHostIf { p: Some(desc) });

    use crate::guestnet::driver::DriveReport;

    let mut saw_on_packet_err = None;
    let mut saw_stats = None;
    drive_once(&mut pio, &mut flows, &mut socks, |r| match r {
        DriveReport::OnPacketError { err } => saw_on_packet_err = Some(err.to_string()),
        DriveReport::Stats(s) => saw_stats = Some(s),
        DriveReport::Pump(_p) => {}
        DriveReport::TxWouldBlock => {}
    })
    .unwrap();

    let err = saw_on_packet_err.expect("expected OnPacketError report");
    assert!(
        err.contains("malformed packet") || err.contains("unsupported") || err.contains("invalid"),
        "unexpected error string: {err}"
    );

    let stats = saw_stats.expect("expected Stats report");
    assert_eq!(stats.packets_rx, 1);
    assert_eq!(stats.packets_on_packet_err, 1);
    assert_eq!(stats.socket_rx_full_drops, 0);
}
