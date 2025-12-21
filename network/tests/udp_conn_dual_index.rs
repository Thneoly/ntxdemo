use ntx_network::socket::udp;

#[test]
fn udp_conn_table_dual_index_lookup() {
    let mut table = udp::Table::default();

    // Insert via connect (no rx packet required).
    let conn = table.connect(
        ntx_network::Ipv4Addr([10, 0, 0, 1]),
        12345,
        ntx_network::Ipv4Addr([10, 0, 0, 2]),
        40000,
        ntx_network::MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]),
        ntx_network::MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]),
        64,
    );

    let key = conn.key;
    let sock_id = key.id;

    assert_eq!(table.len(), 1);

    // Lookup by 4-tuple (Key Eq/Hash ignores id, so using id=0 is fine here).
    let tuple_key = udp::Key {
        id: 0,
        peer_ip: key.peer_ip,
        peer_port: key.peer_port,
        local_ip: key.local_ip,
        local_port: key.local_port,
    };
    let by_tuple = table.peek(&tuple_key).expect("lookup by tuple");
    assert_eq!(by_tuple.key.peer_port, 12345);

    // Lookup by sock_id (secondary index).
    let by_id = table.get_by_sock_id(sock_id).expect("lookup by id");
    assert_eq!(by_id.key.local_port, 40000);

    // Should not create extra entries.
    assert_eq!(table.len(), 1);
}
