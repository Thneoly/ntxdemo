use crate::guestnet::flow::{
    EndpointV4, FlowKey, FlowManager, SocketBindKey, SocketId, TransportProto, flow_key_hash,
};

#[test]
fn lookup_or_create_is_stable_and_updates_last_seen() {
    let mut fm = FlowManager::new();
    fm.set_now_tick(1);

    let key = FlowKey {
        proto: TransportProto::Udp,
        src_ip: [10, 0, 0, 1],
        dst_ip: [10, 0, 0, 2],
        src_port: 1234,
        dst_port: 4321,
    };

    let e1_ptr = {
        let e1 = fm.lookup_or_create(key);
        assert_eq!(e1.last_seen_tick, 1);
        e1 as *mut _
    };

    fm.set_now_tick(2);
    let e2_ptr = {
        let e2 = fm.lookup_or_create(key);
        assert_eq!(e2.last_seen_tick, 2);
        e2 as *mut _
    };

    assert_eq!(fm.len(), 1);
    assert_eq!(e1_ptr, e2_ptr);
}

#[test]
fn bind_socket_and_lookup_socket() {
    let mut fm = FlowManager::new();
    fm.set_now_tick(1);

    let key = FlowKey {
        proto: TransportProto::Tcp,
        src_ip: [192, 168, 0, 10],
        dst_ip: [192, 168, 0, 20],
        src_port: 1111,
        dst_port: 2222,
    };

    assert_eq!(fm.socket_for_inbound_flow(&key), None);

    // Bind local-only.
    fm.bind_socket(
        SocketBindKey {
            proto: TransportProto::Tcp,
            local: EndpointV4 {
                ip: [192, 168, 0, 20],
                port: 2222,
            },
            remote: None,
        },
        SocketId(7),
    );
    assert_eq!(fm.socket_for_inbound_flow(&key), Some(SocketId(7)));
}

#[test]
fn flow_key_hash_is_deterministic() {
    let key = FlowKey {
        proto: TransportProto::Udp,
        src_ip: [1, 2, 3, 4],
        dst_ip: [5, 6, 7, 8],
        src_port: 9,
        dst_port: 10,
    };

    let h1 = flow_key_hash(&key);
    let h2 = flow_key_hash(&key);
    assert_eq!(h1, h2);
}
