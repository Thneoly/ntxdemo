use ntx_network::abr;
use ntx_network::resources::ResourcePoolsConfig;
use ntx_network::resources::{NonSocketResourceValue, ResourceKind, SockId};
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
    // Use the public single-entrypoint API instead of per-pool accessors.
    let (rid, v) = pools
        .acquire_and_pin_non_socket(ResourceKind::Ipv4, "demo", Uuid::new_v4(), None)
        .expect("acquire ipv4");
    let NonSocketResourceValue::Ipv4(ip) = v else {
        panic!("expected ipv4, got {v:?} for rid={rid}")
    };
    assert_eq!(ip.octets(), [10, 0, 0, 2]);

    // MAC range of 3 addresses.
    let mut macs = std::collections::BTreeSet::new();
    for _ in 0..3 {
        let (_rid, v) = pools
            .acquire_and_pin_non_socket(ResourceKind::Mac, "demo", Uuid::new_v4(), None)
            .expect("acquire mac");
        let NonSocketResourceValue::Mac(mac) = v else {
            panic!("expected mac")
        };
        assert!(macs.insert(mac));
    }
    assert_eq!(macs.len(), 3);

    // Port range: 40000..=40002 excluding 40001 => {40000,40002}
    let (_rid1, v1) = pools
        .acquire_and_pin_non_socket(ResourceKind::UdpPort, "demo", Uuid::new_v4(), None)
        .expect("acquire udp port 1");
    let (_rid2, v2) = pools
        .acquire_and_pin_non_socket(ResourceKind::UdpPort, "demo", Uuid::new_v4(), None)
        .expect("acquire udp port 2");
    let NonSocketResourceValue::UdpPort(p1) = v1 else {
        panic!("expected udp port")
    };
    let NonSocketResourceValue::UdpPort(p2) = v2 else {
        panic!("expected udp port")
    };
    assert_ne!(p1, p2);
    assert!(matches!((p1, p2), (40000, 40002) | (40002, 40000)));
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

    // Acquire an IP and pin it to a stable owner; then ensure resolve works.
    let owner = Uuid::new_v4();
    let (rid, v) = pools
        .acquire_and_pin_non_socket(ResourceKind::Ipv4, "demo", owner, None)
        .expect("acquire ipv4");
    let NonSocketResourceValue::Ipv4(ip) = v else {
        panic!("expected ipv4")
    };

    let resolved = pools
        .resolve_non_socket(ResourceKind::Ipv4, &rid)
        .expect("resolve pinned ip");
    assert_eq!(resolved, NonSocketResourceValue::Ipv4(ip));
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

    let (_rid, v) = pools
        .acquire_and_pin_non_socket(ResourceKind::UdpPort, "demo", Uuid::new_v4(), None)
        .expect("acquire udp");
    assert_eq!(v, NonSocketResourceValue::UdpPort(42000));

    let (_rid, v) = pools
        .acquire_and_pin_non_socket(ResourceKind::TcpPort, "demo", Uuid::new_v4(), None)
        .expect("acquire tcp");
    assert_eq!(v, NonSocketResourceValue::TcpPort(43000));
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
    let (_ip_rid, ip_v) = pools
        .acquire_and_pin_non_socket(ResourceKind::Ipv4, "demo", owner, None)
        .expect("acquire ipv4");
    let NonSocketResourceValue::Ipv4(ip) = ip_v else {
        panic!("expected ipv4")
    };

    let (_p1_rid, p1_v) = pools
        .acquire_and_pin_non_socket(ResourceKind::UdpPort, "demo", owner, None)
        .expect("acquire port1");
    let (_p2_rid, p2_v) = pools
        .acquire_and_pin_non_socket(ResourceKind::UdpPort, "demo", owner, None)
        .expect("acquire port2");
    let NonSocketResourceValue::UdpPort(p1) = p1_v else {
        panic!("expected udp port")
    };
    let NonSocketResourceValue::UdpPort(p2) = p2_v else {
        panic!("expected udp port")
    };

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

    // The generic API doesn't expose pool-internal round-robin semantics directly.
    // This behavior is still covered by unit tests closer to the pool implementation;
    // here we just ensure a single owner can acquire multiple UDP ports.
    let cfg: ResourcePoolsConfig = serde_yaml::from_str(yaml).unwrap();
    let mut pools = cfg.build().unwrap();
    let owner = Uuid::new_v4();

    let mut ports = std::collections::BTreeSet::new();
    for _ in 0..3 {
        let (_rid, v) = pools
            .acquire_and_pin_non_socket(ResourceKind::UdpPort, "demo", owner, None)
            .expect("acquire udp port");
        let NonSocketResourceValue::UdpPort(p) = v else {
            panic!("expected udp port")
        };
        assert!(ports.insert(p));
    }
    assert_eq!(ports.len(), 3);
}

#[test]
fn acquire_udp_port_for_registers_resource_id() {
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
    let (rid, port) = {
        let (rid, v) = pools
            .acquire_and_pin_non_socket(ResourceKind::UdpPort, "demo", owner, Some(using_sock_id))
            .expect("acquire udp port");
        let NonSocketResourceValue::UdpPort(p) = v else {
            unreachable!("resource kind/value mismatch")
        };
        (rid, p)
    };

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
    let (rid, _ip) = {
        let (rid, v) = pools
            .acquire_and_pin_non_socket(ResourceKind::Ipv4, "demo", owner, Some(using_sock_id))
            .expect("alloc ipv4");
        let NonSocketResourceValue::Ipv4(ip) = v else {
            unreachable!("resource kind/value mismatch")
        };
        (rid, ip)
    };

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
    let (rid, _mac) = {
        let (rid, v) = pools
            .acquire_and_pin_non_socket(ResourceKind::Mac, "demo", owner, Some(using_sock_id))
            .expect("alloc mac");
        let NonSocketResourceValue::Mac(mac) = v else {
            unreachable!("resource kind/value mismatch")
        };
        (rid, mac)
    };

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
    let (rid, port) = {
        let (rid, v) = pools
            .acquire_and_pin_non_socket(ResourceKind::TcpPort, "demo", owner, Some(using_sock_id))
            .expect("alloc tcp port");
        let NonSocketResourceValue::TcpPort(p) = v else {
            unreachable!("resource kind/value mismatch")
        };
        (rid, p)
    };

    assert_eq!(port, 48000);
    assert_eq!(pools.registry().kind_of(&rid), Some(ResourceKind::TcpPort));
    assert_eq!(pools.registry().owner_of(&rid), Some(owner));
    assert_eq!(pools.registry().using_sock_id_of(&rid), Some(using_sock_id));
}

#[test]
fn acquire_socket_owner_registers_socket_info() {
    let cfg: ResourcePoolsConfig = serde_yaml::from_str("{}").unwrap();
    let mut pools = cfg.build().unwrap();

    let owner = pools.acquire_socket_owner("sock-a");
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

    let owner = pools.acquire_socket_owner("sock-pin");
    let using_sock_id: SockId = 42;

    let ip: std::net::Ipv4Addr = "10.11.0.2".parse().unwrap();
    let ip_rid = pools
        .pin_non_socket_with_id(
            ResourceKind::Ipv4,
            "demo",
            owner,
            NonSocketResourceValue::Ipv4(ip),
            Some(using_sock_id),
        )
        .expect("pin ipv4");
    assert_eq!(pools.registry().kind_of(&ip_rid), Some(ResourceKind::Ipv4));
    assert_eq!(pools.registry().owner_of(&ip_rid), Some(owner));
    assert_eq!(
        pools.registry().using_sock_id_of(&ip_rid),
        Some(using_sock_id)
    );

    let udp_rid = pools
        .pin_non_socket_with_id(
            ResourceKind::UdpPort,
            "demo",
            owner,
            NonSocketResourceValue::UdpPort(49000),
            Some(using_sock_id),
        )
        .expect("pin udp");
    assert_eq!(
        pools.registry().kind_of(&udp_rid),
        Some(ResourceKind::UdpPort)
    );
    assert_eq!(pools.registry().owner_of(&udp_rid), Some(owner));

    let tcp_rid = pools
        .pin_non_socket_with_id(
            ResourceKind::TcpPort,
            "demo",
            owner,
            NonSocketResourceValue::TcpPort(49100),
            Some(using_sock_id),
        )
        .expect("pin tcp");
    assert_eq!(
        pools.registry().kind_of(&tcp_rid),
        Some(ResourceKind::TcpPort)
    );
    assert_eq!(pools.registry().owner_of(&tcp_rid), Some(owner));

    let mac = ntx_network::MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]);
    let mac_rid = pools
        .pin_non_socket_with_id(
            ResourceKind::Mac,
            "demo",
            owner,
            NonSocketResourceValue::Mac(mac),
            Some(using_sock_id),
        )
        .expect("pin mac");
    assert_eq!(pools.registry().kind_of(&mac_rid), Some(ResourceKind::Mac));
    assert_eq!(pools.registry().owner_of(&mac_rid), Some(owner));
}
