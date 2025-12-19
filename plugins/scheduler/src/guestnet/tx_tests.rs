use crate::guestnet::driver::{DriveReport, drive_tx_once};
use crate::guestnet::flow::{EndpointV4, FlowManager, TransportProto};
use crate::guestnet::host_if::{Event, EventKind, PacketDesc, SharedMem, TxError};
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
struct FakeHost {
    tx: Vec<Vec<u8>>,
}

impl HostIf for FakeHost {
    fn poll_packet(&mut self) -> Option<PacketDesc> {
        None
    }

    fn poll_oneoff(&mut self, _interests: &[EventKind]) -> Vec<Event> {
        Vec::new()
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        self.tx.push(frame.to_vec());
        Ok(())
    }

    fn tx_submit_l3_ipv4(&mut self, _packet: &[u8]) -> Result<(), TxError> {
        Err(TxError::Unsupported)
    }
}

#[test]
fn tx_path_socket_send_transport_encode_driver_submit() {
    let shm = TestShm { backing: vec![] };
    let host = FakeHost::default();
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

    // Enqueue one outgoing datagram. This should land in transport txq.
    socks.send(s, b"pong").unwrap();

    let mut saw_stats = None;
    drive_tx_once(&mut pio, &mut flows, &mut socks, |r| {
        if let DriveReport::Stats(s) = r {
            saw_stats = Some(s);
        }
    })
    .unwrap();

    let stats = saw_stats.expect("expected Stats");
    assert_eq!(stats.tx_frames_sent, 1);
    assert_eq!(stats.tx_would_block, 0);

    // Extract what the fake host received.
    let host = pio.host_mut();
    assert_eq!(host.tx.len(), 1);
    let frame = &host.tx[0];

    // Decode the frame to validate encode correctness.
    let (_eth, l3) =
        ntx_network::packet::headers::EthernetHeader::decode(frame).expect("ethernet decode");
    let (ip, l4) = ntx_network::packet::headers::Ipv4Header::decode(l3).expect("ipv4 decode");
    let (udp, payload) = ntx_network::packet::headers::UdpHeader::decode(l4).expect("udp decode");

    assert_eq!(ip.src.octets(), [10, 0, 0, 2]);
    assert_eq!(ip.dst.octets(), [10, 0, 0, 1]);
    assert_eq!(udp.src_port, 2222);
    assert_eq!(udp.dst_port, 1111);
    assert_eq!(payload, b"pong");

    // TX queue should now be empty.
    assert!(matches!(
        socks.poll_tx_frame(s),
        Err(crate::guestnet::socket_api::SocketError::WouldBlock)
    ));
}
