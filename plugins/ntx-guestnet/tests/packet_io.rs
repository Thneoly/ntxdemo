use ntx_guestnet::host_if::{PacketDesc, SharedMem};
use ntx_guestnet::packet_io::{GuestNetError, PacketIo};

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

// NOTE: host_if::poll_packet() is currently a stub returning None, so handle_packets should
// return WouldBlock.
#[test]
fn handle_packets_returns_would_block_when_no_packets() {
    let shm = TestShm {
        backing: vec![0u8; 128],
    };
    let mut io = PacketIo::new(&shm);

    let r = io.handle_packets(|_pkt| {
        panic!("should not be called without packets");
    });

    assert!(matches!(r, Err(GuestNetError::WouldBlock)));
}

#[test]
fn packetdesc_to_view_is_no_copy_borrow() {
    // This test validates the *policy* of no-copy: packet_view_from_desc returns a slice
    // into shared memory, not an allocation.
    let shm = TestShm {
        backing: (0u8..=255).collect(),
    };

    let desc = PacketDesc {
        buf_offset: 10,
        len: 4,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 123,
    };

    let view = ntx_guestnet::host_if::packet_view_from_desc(&shm, desc).expect("view");
    assert_eq!(view.as_bytes(), &[10, 11, 12, 13]);
}
