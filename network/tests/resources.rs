use ntx_network::abr;
use ntx_network::resources::ResourcePoolsConfig;
use ntx_network::resources::{ResourceKind, SockId};
use uuid::Uuid;

#[test]
fn parse_and_build_pools_from_yaml() {
    let yaml = r#"
ipv4:
  - name: demo
    cidr: "10.0.0.0/30"
    exclude: ["10.0.0.1"]
mac:
  - name: demo
    start: "02:00:00:00:00:00"
    end:   "02:00:00:00:00:02"
udp_port:
  - name: demo
    start: 40000
    end: 40002
    exclude: [40001]
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();

    // /30 host range => 10.0.0.1-10.0.0.2, but 10.0.0.1 excluded => only 10.0.0.2
    let ipv4 = pools.ipv4("demo").unwrap();
    assert_eq!(ipv4.len_available(), 1);
    let ip = ipv4.acquire().unwrap();
    assert_eq!(ip.0, [10, 0, 0, 2]);
    assert!(ipv4.acquire().is_none());
    assert!(ipv4.release(ip));
    assert!(ipv4.acquire().is_some());

    // MAC range of 3 addresses.
    let mac = pools.mac("demo").unwrap();
    let a = mac.acquire().unwrap();
    let b = mac.acquire().unwrap();
    let c = mac.acquire().unwrap();
    assert!(mac.acquire().is_none());
    assert!(mac.release(b));
    assert!(mac.acquire().is_some());
    let _ = (a, c);

    // Port range: 40000..=40002 excluding 40001 => {40000,40002}
    let port = pools.udp_port("demo").unwrap();
    let p1 = port.acquire().unwrap();
    let p2 = port.acquire().unwrap();
    assert_ne!(p1, p2);
    assert!(matches!((p1, p2), (40000, 40002) | (40002, 40000)));
    assert!(port.acquire().is_none());
}

#[test]
fn named_pools_and_pin() {
    let yaml = r#"
ipv4:
  - name: demo
    cidr: "10.0.1.0/30"
mac:
  - name: demo
    start: "02:00:00:00:00:10"
    end:   "02:00:00:00:00:11"
port:
  - name: demo
    start: 41000
    end: 41001
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();

    let ip = pools.ipv4("demo").unwrap().acquire().unwrap();
    pools.ipv4("demo").unwrap().release(ip);

    // Pin an IP for an owner.
    let owner = Uuid::new_v4();
    pools.ipv4("demo").unwrap().pin(owner, ip).unwrap();

    // acquire_for should return the pinned ip.
    let ip2 = pools.ipv4("demo").unwrap().acquire_for(&owner).unwrap();
    assert_eq!(ip2, ip);
    assert!(pools.ipv4("demo").unwrap().release(ip2));

    // ip should not return to general pool while pinned.
    // (the pool may still have other free addresses; we only assert the pinned
    // address is not handed out via normal acquire())
    if let Some(other) = pools.ipv4("demo").unwrap().acquire() {
        assert_ne!(other, ip);
    }

    assert!(pools.ipv4("demo").unwrap().unpin_owner(&owner));
    // Now it can be acquired normally again.
    assert!(pools.ipv4("demo").unwrap().acquire().is_some());
}

#[test]
fn tcp_and_udp_port_pools_can_coexist() {
    let yaml = r#"
udp_port:
  - name: demo
    start: 42000
    end: 42000
tcp_port:
  - name: demo
    start: 43000
    end: 43000
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();

    assert_eq!(pools.udp_port("demo").unwrap().acquire().unwrap(), 42000);
    assert_eq!(pools.tcp_port("demo").unwrap().acquire().unwrap(), 43000);
}

#[test]
fn publish_abr_for_owner_includes_all_pinned_ports() {
    let yaml = r#"
ipv4:
  - name: demo
    cidr: "10.9.0.0/30"
udp_port:
  - name: demo
    start: 45000
    end: 45002
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();

    let owner = Uuid::new_v4();

    // Pin one IPv4 and multiple UDP ports for the same owner.
    let ip = pools.ipv4("demo").unwrap().acquire().unwrap();
    pools.ipv4("demo").unwrap().release(ip);
    pools.ipv4("demo").unwrap().pin(owner, ip).unwrap();

    let p1 = pools.udp_port("demo").unwrap().acquire().unwrap();
    let p2 = pools.udp_port("demo").unwrap().acquire().unwrap();
    pools.udp_port("demo").unwrap().release(p1);
    pools.udp_port("demo").unwrap().release(p2);
    pools.udp_port("demo").unwrap().pin(owner, p1).unwrap();
    pools.udp_port("demo").unwrap().pin(owner, p2).unwrap();

    let mut store = abr::BindingStore::default();
    pools.publish_abr_for_owner(&mut store, &owner, abr::BindingOwner::Process { pid: 1 });
    let view = abr::load_view();

    let ip_be = u32::from_be_bytes(ip.octets());
    assert!(view.ipv4.contains_be(ip_be));
    assert!(view.udp_ports.contains_be(ip_be, p1));
    assert!(view.udp_ports.contains_be(ip_be, p2));
}

#[test]
fn acquire_for_round_robins_pinned_ports() {
    let yaml = r#"
udp_port:
  - name: demo
    start: 46000
    end: 46002
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();
    let pool = pools.udp_port("demo").unwrap();

    let owner = Uuid::new_v4();

    // Pin three ports for the same owner.
    let a = pool.acquire().unwrap();
    let b = pool.acquire().unwrap();
    let c = pool.acquire().unwrap();
    pool.release(a);
    pool.release(b);
    pool.release(c);
    pool.pin(owner, a).unwrap();
    pool.pin(owner, b).unwrap();
    pool.pin(owner, c).unwrap();

    // Should cycle through pinned ports.
    let p1 = pool.acquire_for(&owner).unwrap();
    let p2 = pool.acquire_for(&owner).unwrap();
    let p3 = pool.acquire_for(&owner).unwrap();
    assert_ne!(p1, p2);
    assert_ne!(p2, p3);
    assert_ne!(p1, p3);

    // Release one and ensure it can be returned again on subsequent acquire_for calls.
    pool.release(p2);
    let p4 = pool.acquire_for(&owner).unwrap();
    assert_eq!(p4, p2);
}

#[test]
fn alloc_udp_port_for_registers_resource_id() {
    let yaml = r#"
udp_port:
  - name: demo
  start: 47000
  end: 47000
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();

    let owner = Uuid::new_v4();
    let using_sock_id: SockId = 123;
    let (rid, port) = pools
        .alloc_udp_port_for("demo", owner, Some(using_sock_id))
        .expect("alloc udp port");

    assert_eq!(port, 47000);
    assert_eq!(pools.registry().kind_of(&rid), Some(ResourceKind::UdpPort));
    assert_eq!(pools.registry().owner_of(&rid), Some(owner));
    assert_eq!(pools.registry().using_sock_id_of(&rid), Some(using_sock_id));

    // Owner should be able to enumerate resources.
    let owned = pools.registry().resources_of_owner(&owner);
    assert_eq!(owned, vec![rid]);
}

#[test]
fn alloc_ipv4_for_registers_resource_id() {
    let yaml = r#"
ipv4:
  - name: demo
  cidr: "10.10.0.0/30"
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();

    let owner = Uuid::new_v4();
    let using_sock_id: SockId = 7;
    let (rid, _ip) = pools
        .alloc_ipv4_for("demo", owner, Some(using_sock_id))
        .expect("alloc ipv4");

    assert_eq!(pools.registry().kind_of(&rid), Some(ResourceKind::Ipv4));
    assert_eq!(pools.registry().owner_of(&rid), Some(owner));
    assert_eq!(pools.registry().using_sock_id_of(&rid), Some(using_sock_id));
}

#[test]
fn alloc_mac_for_registers_resource_id() {
    let yaml = r#"
mac:
  - name: demo
  start: "02:00:00:00:00:01"
  end:   "02:00:00:00:00:01"
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();

    let owner = Uuid::new_v4();
    let using_sock_id: SockId = 8;
    let (rid, _mac) = pools
        .alloc_mac_for("demo", owner, Some(using_sock_id))
        .expect("alloc mac");

    assert_eq!(pools.registry().kind_of(&rid), Some(ResourceKind::Mac));
    assert_eq!(pools.registry().owner_of(&rid), Some(owner));
    assert_eq!(pools.registry().using_sock_id_of(&rid), Some(using_sock_id));
}

#[test]
fn alloc_tcp_port_for_registers_resource_id() {
    let yaml = r#"
tcp_port:
  - name: demo
  start: 48000
  end: 48000
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();

    let owner = Uuid::new_v4();
    let using_sock_id: SockId = 9;
    let (rid, port) = pools
        .alloc_tcp_port_for("demo", owner, Some(using_sock_id))
        .expect("alloc tcp port");

    assert_eq!(port, 48000);
    assert_eq!(pools.registry().kind_of(&rid), Some(ResourceKind::TcpPort));
    assert_eq!(pools.registry().owner_of(&rid), Some(owner));
    assert_eq!(pools.registry().using_sock_id_of(&rid), Some(using_sock_id));
}

#[test]
fn alloc_socket_owner_registers_socket_info() {
    let cfg: ResourcePoolsConfig = serde_yaml::from_str("{}").unwrap();
    let mut pools = cfg.build().unwrap();

    let owner = pools.alloc_socket_owner("sock-a");
    assert_eq!(pools.registry().kind_of(&owner), Some(ResourceKind::Socket));
    assert_eq!(
        pools.registry().socket_info(&owner).unwrap().name,
        "sock-a".to_string()
    );
}

#[test]
fn pin_with_id_registers_resource() {
    let yaml = r#"
ipv4:
    - name: demo
        cidr: "10.11.0.0/30"
udp_port:
    - name: demo
        start: 49000
        end: 49000
tcp_port:
    - name: demo
        start: 49100
        end: 49100
mac:
    - name: demo
        start: "02:00:00:00:00:02"
        end:   "02:00:00:00:00:02"
"#;

    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();

    let owner = pools.alloc_socket_owner("sock-pin");
    let using_sock_id: SockId = 42;

    let ip: std::net::Ipv4Addr = "10.11.0.2".parse().unwrap();
    let ip_rid = pools
        .pin_ipv4_with_id("demo", owner, ip, Some(using_sock_id))
        .expect("pin ipv4");
    assert_eq!(pools.registry().kind_of(&ip_rid), Some(ResourceKind::Ipv4));
    assert_eq!(pools.registry().owner_of(&ip_rid), Some(owner));
    assert_eq!(
        pools.registry().using_sock_id_of(&ip_rid),
        Some(using_sock_id)
    );

    let udp_rid = pools
        .pin_udp_port_with_id("demo", owner, 49000, Some(using_sock_id))
        .expect("pin udp");
    assert_eq!(
        pools.registry().kind_of(&udp_rid),
        Some(ResourceKind::UdpPort)
    );
    assert_eq!(pools.registry().owner_of(&udp_rid), Some(owner));

    let tcp_rid = pools
        .pin_tcp_port_with_id("demo", owner, 49100, Some(using_sock_id))
        .expect("pin tcp");
    assert_eq!(
        pools.registry().kind_of(&tcp_rid),
        Some(ResourceKind::TcpPort)
    );
    assert_eq!(pools.registry().owner_of(&tcp_rid), Some(owner));

    let mac = ntx_network::MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]);
    let mac_rid = pools
        .pin_mac_with_id("demo", owner, mac, Some(using_sock_id))
        .expect("pin mac");
    assert_eq!(pools.registry().kind_of(&mac_rid), Some(ResourceKind::Mac));
    assert_eq!(pools.registry().owner_of(&mac_rid), Some(owner));
}
