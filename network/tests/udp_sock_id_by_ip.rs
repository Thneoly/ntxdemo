use ntx_network::{ConnTableConfig, Ipv4Addr, MacAddr, socket::udp};

#[test]
fn udp_sock_id_lookup_by_local_ip_and_port() {
    let mut table = udp::Table::new(ConnTableConfig {
        max_entries: 16,
        ttl: None,
    });

    let peer_ip = Ipv4Addr([10, 0, 0, 1]);
    let local_ip = Ipv4Addr([10, 0, 0, 2]);
    let peer_port = 1111;
    let local_port = 2222;

    let peer_mac = MacAddr([0, 1, 2, 3, 4, 5]);
    let local_mac = MacAddr([6, 7, 8, 9, 10, 11]);

    let conn = table.connect(
        peer_ip, peer_port, local_ip, local_port, peer_mac, local_mac, 64,
    );
    let sock_id = conn.key.id;

    assert_eq!(table.sock_id_for_local(local_ip, None), Some(sock_id));
    assert_eq!(
        table.sock_id_for_local(local_ip, Some(local_port)),
        Some(sock_id)
    );
    assert_eq!(table.sock_id_for_local(local_ip, Some(9999)), None);
    assert_eq!(table.sock_id_for_local(Ipv4Addr([10, 0, 0, 3]), None), None);
}
