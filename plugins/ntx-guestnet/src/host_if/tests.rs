use super::*;

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

#[test]
fn packet_view_is_no_copy_slice() {
    let shm = TestShm {
        backing: (0u8..=255).collect(),
    };

    let desc = PacketDesc {
        buf_offset: 10,
        len: 5,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 42,
    };

    let view = packet_view_from_desc(&shm, desc).expect("view");
    assert_eq!(view.as_bytes(), &[10, 11, 12, 13, 14]);
}

#[test]
fn packet_view_out_of_bounds_returns_none() {
    let shm = TestShm {
        backing: vec![1, 2, 3],
    };
    let desc = PacketDesc {
        buf_offset: 1,
        len: 10,
        l3_proto: 4,
        l4_proto: 17,
        flow_hash: 1,
    };
    assert!(packet_view_from_desc(&shm, desc).is_none());
}
