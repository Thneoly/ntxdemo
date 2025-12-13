use std::fs;

use ntx::network::Ipv4Addr;
use ntx::network::traffic::scenario::{expand_dst_ips, load_scenario};

#[test]
fn scenario_loads_yaml() {
    let dir = std::env::temp_dir();
    let path = dir.join("ntx_scenario_test.yaml");

    let yaml = r#"
version: 1
iface: eno1
dst_ip_file: dst_ips.txt
src_ip: 192.168.1.10
src_port: 50000
dst_port: 10001
payload: "hello"
pps: 123
count: 456
arp:
  enabled: true
  timeout_ms: 111
  ttl_s: 222
rr:
  enabled: true
  timeout_ms: 333
  poll_budget: 444
"#;

    fs::write(&path, yaml).unwrap();
    let sc = load_scenario(&path).unwrap();

    assert_eq!(sc.version, 1);
    assert_eq!(sc.iface.as_deref(), Some("eno1"));
    assert_eq!(sc.dst_ip_file.as_deref(), Some("dst_ips.txt"));
    assert_eq!(sc.src_ip.as_deref(), Some("192.168.1.10"));
    assert_eq!(sc.src_port, Some(50000));
    assert_eq!(sc.pps, Some(123));
    assert_eq!(sc.arp.enabled, Some(true));
    assert_eq!(sc.rr.poll_budget, Some(444));

    let _ = fs::remove_file(&path);
}

#[test]
fn scenario_dst_ips_expand_single_cidr_range() {
    let entries = vec![
        "10.0.0.1".to_string(),
        "10.0.1.0/30".to_string(), // 4 ips
        "10.0.2.10-10.0.2.12".to_string(),
    ];

    let ips = expand_dst_ips(&entries, 1024).unwrap();
    assert_eq!(ips[0], Ipv4Addr([10, 0, 0, 1]));
    assert!(ips.contains(&Ipv4Addr([10, 0, 1, 0])));
    assert!(ips.contains(&Ipv4Addr([10, 0, 1, 3])));
    assert!(ips.contains(&Ipv4Addr([10, 0, 2, 10])));
    assert!(ips.contains(&Ipv4Addr([10, 0, 2, 12])));
}

#[test]
fn scenario_dst_ips_reject_bad_range() {
    let entries = vec!["10.0.0.10-10.0.0.1".to_string()];
    let err = expand_dst_ips(&entries, 1024).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("range") || msg.contains("start"));
}

#[test]
fn scenario_dst_ips_guard_max() {
    // /16 expands to 65536, so max=1000 should fail.
    let entries = vec!["10.1.0.0/16".to_string()];
    let err = expand_dst_ips(&entries, 1000).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("too many") || msg.contains("refusing"));
}

#[test]
fn scenario_dst_ips_expand_accepts_comma_split_style() {
    // CLI allows comma-separated specs; expansion itself should treat each entry independently.
    let entries = vec!["10.0.0.1".to_string(), "10.0.0.2".to_string()];
    let ips = expand_dst_ips(&entries, 16).unwrap();
    assert_eq!(ips, vec![Ipv4Addr([10, 0, 0, 1]), Ipv4Addr([10, 0, 0, 2])]);
}
