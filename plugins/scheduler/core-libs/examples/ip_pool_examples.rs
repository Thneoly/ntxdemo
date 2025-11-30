// IP Pool Usage Examples
//
// This file demonstrates various use cases of the IP pool management library.

use scheduler_core::{IpPool, IpPoolError, IpRange, ResourceType};
use std::net::{IpAddr, Ipv4Addr};

fn main() -> Result<(), IpPoolError> {
    println!("=== IP Pool Management Examples ===\n");

    // Example 1: Basic pool creation and allocation
    example_1_basic_usage()?;

    // Example 2: Multi-tenant scenario
    example_2_multi_tenant()?;

    // Example 3: MAC address binding
    example_3_mac_binding()?;

    // Example 4: Container/Pod management
    example_4_container_management()?;

    // Example 5: Reserved IPs and specific allocation
    example_5_reserved_ips()?;

    Ok(())
}

fn example_1_basic_usage() -> Result<(), IpPoolError> {
    println!("--- Example 1: Basic Usage ---");

    let mut pool = IpPool::new("basic-pool");
    pool.add_cidr_range("192.168.1.0/24")?;

    // Allocate IPs
    let ip1 = pool.allocate("default", "vm-001", ResourceType::Vm("vm-001".to_string()))?;
    let ip2 = pool.allocate("default", "vm-002", ResourceType::Vm("vm-002".to_string()))?;

    println!("Allocated IP for vm-001: {}", ip1);
    println!("Allocated IP for vm-002: {}", ip2);

    // Check stats
    let stats = pool.stats();
    println!(
        "Pool stats: {} allocated out of {}",
        stats.allocated, stats.total
    );

    // Release an IP
    pool.release_by_subid("default", "vm-001")?;
    println!("Released IP for vm-001");

    println!();
    Ok(())
}

fn example_2_multi_tenant() -> Result<(), IpPoolError> {
    println!("--- Example 2: Multi-Tenant Scenario ---");

    let mut pool = IpPool::new("multi-tenant-pool");
    pool.add_cidr_range("10.0.0.0/16")?;

    // Tenant A resources
    pool.allocate(
        "tenant-a",
        "web-01",
        ResourceType::Vm("web-server-1".to_string()),
    )?;
    pool.allocate(
        "tenant-a",
        "db-01",
        ResourceType::Vm("database-1".to_string()),
    )?;
    pool.allocate(
        "tenant-a",
        "cache-01",
        ResourceType::Container("redis-cache".to_string()),
    )?;

    // Tenant B resources
    pool.allocate(
        "tenant-b",
        "api-01",
        ResourceType::Container("api-gateway".to_string()),
    )?;
    pool.allocate(
        "tenant-b",
        "worker-01",
        ResourceType::Pod("worker-pod-abc".to_string()),
    )?;

    // Query tenant usage
    let tenant_a_ips = pool.list_by_subinstance("tenant-a");
    let tenant_b_ips = pool.list_by_subinstance("tenant-b");

    println!("Tenant A has {} IPs allocated:", tenant_a_ips.len());
    for ip in &tenant_a_ips {
        if let Some(binding) = pool.get_binding(ip) {
            println!(
                "  {} -> {} ({})",
                ip,
                binding.subid,
                binding.subtype.type_name()
            );
        }
    }

    println!("Tenant B has {} IPs allocated:", tenant_b_ips.len());
    for ip in &tenant_b_ips {
        if let Some(binding) = pool.get_binding(ip) {
            println!(
                "  {} -> {} ({})",
                ip,
                binding.subid,
                binding.subtype.type_name()
            );
        }
    }

    // Clean up tenant A
    let released = pool.release_by_subinstance("tenant-a");
    println!("Released {} IPs from tenant-a", released.len());

    println!();
    Ok(())
}

fn example_3_mac_binding() -> Result<(), IpPoolError> {
    println!("--- Example 3: MAC Address Binding ---");

    let mut pool = IpPool::new("mac-pool");
    pool.add_cidr_range("172.16.0.0/16")?;

    // Network devices with MAC addresses
    let devices = vec![
        ("device-001", "AA:BB:CC:DD:EE:01"),
        ("device-002", "AA:BB:CC:DD:EE:02"),
        ("device-003", "AA:BB:CC:DD:EE:03"),
    ];

    // Allocate IPs for devices
    for (device_id, mac) in &devices {
        let ip = pool.allocate(
            "network-segment-1",
            *device_id,
            ResourceType::Mac(mac.to_string()),
        )?;
        println!("Device {} (MAC: {}) -> IP: {}", device_id, mac, ip);
    }

    // Later, find IP by MAC address
    let mac_to_find = "AA:BB:CC:DD:EE:02";
    if let Some(ip) = pool.find_by_resource("mac", mac_to_find) {
        println!("\nFound IP {} for MAC {}", ip, mac_to_find);

        if let Some(binding) = pool.get_binding(ip) {
            println!("Device ID: {}", binding.subid);
            println!("Allocated at: {}", binding.allocated_at);
        }
    }

    println!();
    Ok(())
}

fn example_4_container_management() -> Result<(), IpPoolError> {
    println!("--- Example 4: Container/Pod Management ---");

    let mut pool = IpPool::new("k8s-pod-network");
    pool.add_cidr_range("10.244.0.0/16")?;

    // Simulate Kubernetes pod lifecycle
    let namespace = "production";

    // Create pods
    let pods = vec![
        ("nginx-deployment-abc123", "nginx-web"),
        ("mysql-stateful-xyz789", "mysql-db"),
        ("redis-deployment-def456", "redis-cache"),
    ];

    for (pod_name, pod_type) in &pods {
        let ip = pool.allocate(
            namespace,
            *pod_name,
            ResourceType::Pod(pod_name.to_string()),
        )?;
        println!("Pod {} ({}) allocated IP: {}", pod_name, pod_type, ip);
    }

    // Simulate pod deletion
    let pod_to_delete = "nginx-deployment-abc123";
    let binding = pool.release_by_subid(namespace, pod_to_delete)?;
    println!(
        "\nDeleted pod {}, released IP {}",
        pod_to_delete, binding.ip
    );

    // List remaining pods
    let remaining_ips = pool.list_by_subinstance(namespace);
    println!("\nRemaining pods in namespace '{}':", namespace);
    for ip in remaining_ips {
        if let Some(binding) = pool.get_binding(&ip) {
            println!("  {} -> {}", ip, binding.subid);
        }
    }

    println!();
    Ok(())
}

fn example_5_reserved_ips() -> Result<(), IpPoolError> {
    println!("--- Example 5: Reserved IPs and Specific Allocation ---");

    let mut pool = IpPool::new("production-pool");
    pool.add_cidr_range("192.168.100.0/24")?;

    // Reserve gateway and special IPs
    let gateway = IpAddr::V4(Ipv4Addr::new(192, 168, 100, 1));
    let dns = IpAddr::V4(Ipv4Addr::new(192, 168, 100, 2));
    let broadcast = IpAddr::V4(Ipv4Addr::new(192, 168, 100, 255));

    pool.reserve_ip(gateway);
    pool.reserve_ip(dns);
    pool.reserve_ip(broadcast);

    println!("Reserved IPs: {}, {}, {}", gateway, dns, broadcast);

    // Allocate specific IP for critical service
    let lb_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 100, 10));
    pool.allocate_specific(
        lb_ip,
        "infrastructure",
        "load-balancer",
        ResourceType::Vm("lb-primary".to_string()),
    )?;
    println!("Allocated specific IP {} for load balancer", lb_ip);

    // Auto-allocate for other services
    let ip1 = pool.allocate(
        "infrastructure",
        "web-01",
        ResourceType::Vm("web-1".to_string()),
    )?;
    let ip2 = pool.allocate(
        "infrastructure",
        "web-02",
        ResourceType::Vm("web-2".to_string()),
    )?;

    println!("Auto-allocated IP {} for web-01", ip1);
    println!("Auto-allocated IP {} for web-02", ip2);

    // Show statistics
    let stats = pool.stats();
    println!("\nPool Statistics:");
    println!("  Total IPs: {}", stats.total);
    println!("  Allocated: {}", stats.allocated);
    println!("  Reserved: {}", stats.reserved);
    println!("  Available: {}", stats.available);

    println!();
    Ok(())
}
