# IP Pool Management Library

## Overview

The IP Pool Management library provides comprehensive IP address allocation and management capabilities for the scheduler core. It supports:

- IP range definition with CIDR notation
- Dynamic IP allocation and release
- Binding IPs to resources with subinstance/subid/subtype
- Multiple resource type support (MAC, VM, Container, Pod, Custom)
- Reserved IP management
- Pool statistics and monitoring

## Quick Start

```rust
use scheduler_core::{IpPool, ResourceType};
use std::net::IpAddr;

// Create a new IP pool
let mut pool = IpPool::new("my-pool");

// Add IP ranges
pool.add_cidr_range("192.168.1.0/24")?;
pool.add_cidr_range("10.0.0.0/16")?;

// Allocate an IP for a MAC address
let ip = pool.allocate(
    "instance-01",
    "vm-001", 
    ResourceType::Mac("00:11:22:33:44:55".to_string())
)?;

println!("Allocated IP: {}", ip);

// Find IP by subinstance and subid
if let Some(ip) = pool.find_by_subid("instance-01", "vm-001") {
    println!("Found IP: {}", ip);
}

// Release the IP
pool.release_by_subid("instance-01", "vm-001")?;
```

## Core Concepts

### 1. IP Ranges

IP ranges define the pool of available addresses. You can create ranges in several ways:

```rust
use std::net::{IpAddr, Ipv4Addr};

// Method 1: From start/end addresses
let start = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
let end = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 254));
let range = IpRange::new(start, end)?;

// Method 2: From CIDR notation (recommended)
let range = IpRange::from_cidr("192.168.1.0/24")?;
pool.add_range(range);

// Method 3: Add directly to pool
pool.add_cidr_range("10.0.0.0/16")?;
```

### 2. Resource Types

The library supports multiple resource types for IP binding:

```rust
// MAC address binding
ResourceType::Mac("AA:BB:CC:DD:EE:FF".to_string())

// Virtual machine binding
ResourceType::Vm("vm-12345".to_string())

// Container binding
ResourceType::Container("container-abc".to_string())

// Kubernetes Pod binding
ResourceType::Pod("nginx-pod-xyz".to_string())

// Custom resource type
ResourceType::Custom("custom-resource-id".to_string())
```

### 3. IP Binding

Each allocated IP is bound to a resource with three identifiers:

- **subinstance**: Instance or namespace identifier (e.g., "tenant-01", "cluster-west")
- **subid**: Resource identifier within the instance (e.g., "vm-001", "pod-12")
- **subtype**: Resource type and its specific identifier (e.g., MAC address)

```rust
let binding = IpBinding::new(
    ip_address,
    "instance-01",  // subinstance
    "vm-001",       // subid
    ResourceType::Mac("00:11:22:33:44:55".to_string())
);

// Add optional metadata
let binding = binding
    .with_metadata("region", "us-west")
    .with_metadata("zone", "az1");
```

## Usage Examples

### Basic Pool Management

```rust
// Create and configure pool
let mut pool = IpPool::new("production-pool");
pool.add_cidr_range("192.168.0.0/16")?;

// Reserve gateway and broadcast addresses
pool.reserve_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)));
pool.reserve_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 255, 255)));

// Get pool statistics
let stats = pool.stats();
println!("Pool: {}", stats.name);
println!("Total IPs: {}", stats.total);
println!("Allocated: {}", stats.allocated);
println!("Reserved: {}", stats.reserved);
println!("Available: {}", stats.available);
```

### Allocating IPs

```rust
// Auto-allocate (pool picks next available IP)
let ip1 = pool.allocate(
    "tenant-a",
    "web-server-1",
    ResourceType::Vm("vm-web-01".to_string())
)?;

// Allocate specific IP
let desired_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
pool.allocate_specific(
    desired_ip,
    "tenant-a",
    "db-server-1",
    ResourceType::Container("mysql-primary".to_string())
)?;

// Allocating with same subinstance+subid returns the same IP
let ip1_again = pool.allocate(
    "tenant-a",
    "web-server-1",
    ResourceType::Vm("vm-web-01".to_string())
)?;
assert_eq!(ip1, ip1_again);
```

### Finding IPs

```rust
// Find by subinstance and subid
if let Some(ip) = pool.find_by_subid("tenant-a", "web-server-1") {
    println!("Web server IP: {}", ip);
}

// Find by resource type and identifier
if let Some(ip) = pool.find_by_resource("mac", "00:11:22:33:44:55") {
    println!("MAC address IP: {}", ip);
}

// Get binding details
if let Some(binding) = pool.get_binding(&ip) {
    println!("Subinstance: {}", binding.subinstance);
    println!("Subid: {}", binding.subid);
    println!("Resource: {:?}", binding.subtype);
    println!("Allocated at: {}", binding.allocated_at);
}

// List all IPs for a subinstance
let ips = pool.list_by_subinstance("tenant-a");
println!("Tenant A has {} IPs", ips.len());
```

### Releasing IPs

```rust
// Release by IP address
let binding = pool.release_by_ip(&ip)?;
println!("Released IP {} from {}", binding.ip, binding.subinstance);

// Release by subinstance and subid
pool.release_by_subid("tenant-a", "web-server-1")?;

// Release all IPs for a subinstance
let released = pool.release_by_subinstance("tenant-a");
println!("Released {} IPs from tenant-a", released.len());
```

### Multi-Tenant Scenario

```rust
let mut pool = IpPool::new("multi-tenant");
pool.add_cidr_range("10.0.0.0/16")?;

// Tenant A allocations
let vm1 = pool.allocate("tenant-a", "vm-01", 
    ResourceType::Mac("00:00:00:00:00:01".to_string()))?;
let vm2 = pool.allocate("tenant-a", "vm-02", 
    ResourceType::Mac("00:00:00:00:00:02".to_string()))?;

// Tenant B allocations
let pod1 = pool.allocate("tenant-b", "nginx-01", 
    ResourceType::Pod("nginx-deployment-abc".to_string()))?;
let pod2 = pool.allocate("tenant-b", "redis-01", 
    ResourceType::Pod("redis-statefulset-xyz".to_string()))?;

// Query tenant usage
let tenant_a_ips = pool.list_by_subinstance("tenant-a");
let tenant_b_ips = pool.list_by_subinstance("tenant-b");

println!("Tenant A: {} IPs", tenant_a_ips.len());
println!("Tenant B: {} IPs", tenant_b_ips.len());

// Clean up tenant
pool.release_by_subinstance("tenant-a");
```

### MAC Address Binding

```rust
// Allocate IP for a MAC address
let mac = "AA:BB:CC:DD:EE:FF".to_string();
let ip = pool.allocate(
    "network-segment-1",
    "device-001",
    ResourceType::Mac(mac.clone())
)?;

// Later, find the IP by MAC address
if let Some(ip) = pool.find_by_resource("mac", &mac) {
    println!("Device with MAC {} has IP {}", mac, ip);
}

// Get full binding details
if let Some(binding) = pool.get_binding(&ip) {
    if let ResourceType::Mac(device_mac) = &binding.subtype {
        println!("Confirmed MAC: {}", device_mac);
    }
}
```

### Container/Pod Management

```rust
// Kubernetes-style pod IP management
let mut pool = IpPool::new("k8s-pod-network");
pool.add_cidr_range("10.244.0.0/16")?;

// Allocate IP for pod
let pod_name = "nginx-deployment-abc123";
let pod_ip = pool.allocate(
    "namespace-production",
    pod_name,
    ResourceType::Pod(pod_name.to_string())
)?;

// When pod is deleted, release the IP
pool.release_by_subid("namespace-production", pod_name)?;

// Clean up entire namespace
pool.release_by_subinstance("namespace-production");
```

## API Reference

### `IpPool`

- `new(name)` - Create a new IP pool
- `add_range(range)` - Add an IP range
- `add_cidr_range(cidr)` - Add a CIDR range (e.g., "192.168.1.0/24")
- `reserve_ip(ip)` - Mark an IP as reserved
- `unreserve_ip(ip)` - Remove IP reservation
- `allocate(subinstance, subid, subtype)` - Auto-allocate an IP
- `allocate_specific(ip, subinstance, subid, subtype)` - Allocate specific IP
- `release_by_ip(ip)` - Release an IP address
- `release_by_subid(subinstance, subid)` - Release by identifiers
- `release_by_subinstance(subinstance)` - Release all IPs for instance
- `get_binding(ip)` - Get binding information for an IP
- `find_by_subid(subinstance, subid)` - Find IP by identifiers
- `find_by_resource(type, identifier)` - Find IP by resource
- `list_by_subinstance(subinstance)` - List all IPs for instance
- `list_bindings()` - Get all bindings
- `stats()` - Get pool statistics

### `IpRange`

- `new(start, end)` - Create range from start/end IPs
- `from_cidr(cidr)` - Create range from CIDR notation
- `contains(ip)` - Check if IP is in range
- `iter()` - Iterate over all IPs in range

### `ResourceType`

- `Mac(String)` - MAC address
- `Vm(String)` - Virtual machine ID
- `Container(String)` - Container ID
- `Pod(String)` - Kubernetes pod ID
- `Custom(String)` - Custom resource type

### `IpBinding`

- `new(ip, subinstance, subid, subtype)` - Create new binding
- `with_metadata(key, value)` - Add metadata to binding
- Fields: `ip`, `subinstance`, `subid`, `subtype`, `allocated_at`, `metadata`

### `PoolStats`

- Fields: `name`, `total`, `allocated`, `reserved`, `available`

## Error Handling

The library uses `IpPoolError` for error reporting:

```rust
match pool.allocate("inst", "id", ResourceType::Vm("vm1".to_string())) {
    Ok(ip) => println!("Allocated: {}", ip),
    Err(IpPoolError::PoolFull) => println!("No IPs available"),
    Err(IpPoolError::InvalidIpAddress(addr)) => println!("Invalid: {}", addr),
    Err(IpPoolError::IpAlreadyAllocated(ip)) => println!("Already allocated: {}", ip),
    Err(e) => println!("Error: {}", e),
}
```

Error types:
- `InvalidIpAddress(String)` - Invalid IP format
- `InvalidRange(String)` - Invalid IP range
- `IpNotAvailable(String)` - IP cannot be allocated
- `IpNotFound(String)` - IP not in pool
- `IpAlreadyAllocated(String)` - IP already in use
- `BindingNotFound(String)` - No binding found
- `PoolFull` - No IPs available
- `InvalidSubnet(String)` - Invalid CIDR notation

## Best Practices

1. **Use CIDR notation** for range definition - it's clearer and less error-prone
2. **Reserve special IPs** (gateway, broadcast) immediately after creating the pool
3. **Use consistent subinstance naming** for easier management and cleanup
4. **Include metadata** in bindings for additional context
5. **Release by subinstance** when cleaning up entire tenants/namespaces
6. **Check pool stats regularly** to monitor utilization
7. **Use resource-specific lookups** (find_by_resource) for MAC/identifier queries

## Testing

The module includes comprehensive tests:

```bash
cargo test ip::
```

Tests cover:
- IP range creation and validation
- CIDR notation parsing
- IP allocation and idempotency
- IP release operations
- Resource binding and lookup
- Pool statistics
- Edge cases and error conditions
