use ntx_network::abr;
use ntx_network::resources::ResourcePoolsConfig;

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
    pools.ipv4("demo").unwrap().pin("comp-a", ip).unwrap();

    // acquire_for should return the pinned ip.
    let ip2 = pools.ipv4("demo").unwrap().acquire_for("comp-a").unwrap();
    assert_eq!(ip2, ip);
    assert!(pools.ipv4("demo").unwrap().release(ip2));

    // ip should not return to general pool while pinned.
    // (the pool may still have other free addresses; we only assert the pinned
    // address is not handed out via normal acquire())
    if let Some(other) = pools.ipv4("demo").unwrap().acquire() {
        assert_ne!(other, ip);
    }

    assert!(pools.ipv4("demo").unwrap().unpin_owner("comp-a"));
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

    // Pin one IPv4 and multiple UDP ports for the same owner.
    let ip = pools.ipv4("demo").unwrap().acquire().unwrap();
    pools.ipv4("demo").unwrap().release(ip);
    pools.ipv4("demo").unwrap().pin("comp-a", ip).unwrap();

    let p1 = pools.udp_port("demo").unwrap().acquire().unwrap();
    let p2 = pools.udp_port("demo").unwrap().acquire().unwrap();
    pools.udp_port("demo").unwrap().release(p1);
    pools.udp_port("demo").unwrap().release(p2);
    pools.udp_port("demo").unwrap().pin("comp-a", p1).unwrap();
    pools.udp_port("demo").unwrap().pin("comp-a", p2).unwrap();

    let mut store = abr::BindingStore::default();
    pools.publish_abr_for_owner(&mut store, "comp-a", abr::BindingOwner::Process { pid: 1 });
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

    // Pin three ports for the same owner.
    let a = pool.acquire().unwrap();
    let b = pool.acquire().unwrap();
    let c = pool.acquire().unwrap();
    pool.release(a);
    pool.release(b);
    pool.release(c);
    pool.pin("comp-a", a).unwrap();
    pool.pin("comp-a", b).unwrap();
    pool.pin("comp-a", c).unwrap();

    // Should cycle through pinned ports.
    let p1 = pool.acquire_for("comp-a").unwrap();
    let p2 = pool.acquire_for("comp-a").unwrap();
    let p3 = pool.acquire_for("comp-a").unwrap();
    assert_ne!(p1, p2);
    assert_ne!(p2, p3);
    assert_ne!(p1, p3);

    // Release one and ensure it can be returned again on subsequent acquire_for calls.
    pool.release(p2);
    let p4 = pool.acquire_for("comp-a").unwrap();
    assert_eq!(p4, p2);
}
