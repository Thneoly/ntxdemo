# IP 池管理 - 快速开始

## 安装

在 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
scheduler-core = { path = "../core-libs" }
```

## 5 分钟快速上手

### 1. 创建池并添加 IP 范围

```rust
use scheduler_core::IpPool;

let mut pool = IpPool::new("my-pool");

// 使用 CIDR 表示法添加范围（推荐）
pool.add_cidr_range("192.168.1.0/24").unwrap();
```

### 2. 分配 IP

```rust
use scheduler_core::ResourceType;

// 为虚拟机分配 IP
let vm_ip = pool.allocate(
    "tenant-a",     // 租户/实例
    "vm-001",       // 资源 ID
    ResourceType::Vm("my-vm-001".to_string())
).unwrap();

println!("VM IP: {}", vm_ip);  // 例如: 192.168.1.0
```

### 3. 查找 IP

```rust
// 通过租户和资源 ID 查找
let found_ip = pool.find_by_subid("tenant-a", "vm-001");
assert!(found_ip.is_some());

// 通过资源类型查找
let by_resource = pool.find_by_resource("vm", "my-vm-001");
assert!(by_resource.is_some());
```

### 4. 释放 IP

```rust
// 方式 1: 通过 IP 地址释放
pool.release_by_ip(&vm_ip).unwrap();

// 方式 2: 通过租户和资源 ID 释放
pool.release_by_subid("tenant-a", "vm-001").unwrap();

// 方式 3: 释放整个租户的所有 IP
let released = pool.release_by_subinstance("tenant-a");
println!("释放了 {} 个 IP", released.len());
```

### 5. 查看统计

```rust
let stats = pool.stats();
println!("总共: {}", stats.total);
println!("已分配: {}", stats.allocated);
println!("可用: {}", stats.available);
```

## 常见场景

### MAC 地址绑定

```rust
let ip = pool.allocate(
    "network-1",
    "device-01",
    ResourceType::Mac("00:11:22:33:44:55".to_string())
).unwrap();

// 稍后通过 MAC 查找
let found = pool.find_by_resource("mac", "00:11:22:33:44:55");
```

### 容器 IP 管理

```rust
// 容器启动
let container_ip = pool.allocate(
    "project-alpha",
    "nginx-container",
    ResourceType::Container("container-abc123".to_string())
).unwrap();

// 容器停止
pool.release_by_subid("project-alpha", "nginx-container").unwrap();
```

### Kubernetes Pod 网络

```rust
// Pod 创建
let pod_ip = pool.allocate(
    "namespace-prod",
    "nginx-pod-xyz",
    ResourceType::Pod("nginx-deployment-xyz".to_string())
).unwrap();

// Pod 删除
pool.release_by_subid("namespace-prod", "nginx-pod-xyz").unwrap();

// 删除整个 namespace
pool.release_by_subinstance("namespace-prod");
```

### 保留特殊 IP

```rust
use std::net::{IpAddr, Ipv4Addr};

pool.add_cidr_range("10.0.0.0/24").unwrap();

// 保留网关
let gateway = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
pool.reserve_ip(gateway);

// 保留 DNS
let dns = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
pool.reserve_ip(dns);

// 现在这些 IP 不会被自动分配
```

## 完整示例

```rust
use scheduler_core::{IpPool, ResourceType};

fn main() {
    // 创建池
    let mut pool = IpPool::new("production");
    pool.add_cidr_range("172.16.0.0/16").unwrap();
    
    // 多租户场景
    
    // 租户 A
    let a_web = pool.allocate("tenant-a", "web", 
        ResourceType::Vm("web-server".to_string())).unwrap();
    let a_db = pool.allocate("tenant-a", "db",
        ResourceType::Vm("database".to_string())).unwrap();
    
    // 租户 B
    let b_api = pool.allocate("tenant-b", "api",
        ResourceType::Container("api-gateway".to_string())).unwrap();
    
    // 查看租户 A 的 IP
    let tenant_a_ips = pool.list_by_subinstance("tenant-a");
    println!("租户 A 有 {} 个 IP", tenant_a_ips.len());
    
    // 查看池状态
    let stats = pool.stats();
    println!("使用率: {}/{}", stats.allocated, stats.total);
    
    // 清理租户 A
    pool.release_by_subinstance("tenant-a");
    println!("已清理租户 A");
}
```

## 下一步

- 📖 阅读完整文档: [doc/IP_POOL_USAGE.md](../doc/IP_POOL_USAGE.md)
- 💻 查看更多示例: [examples/ip_pool_examples.rs](../examples/ip_pool_examples.rs)
- 🔧 API 参考: [doc/IP_MODULE_SUMMARY.md](../doc/IP_MODULE_SUMMARY.md)

## 错误处理

```rust
use scheduler_core::IpPoolError;

match pool.allocate("inst", "id", ResourceType::Vm("vm".into())) {
    Ok(ip) => println!("成功: {}", ip),
    Err(IpPoolError::PoolFull) => println!("池已满"),
    Err(e) => println!("错误: {}", e),
}
```

## 最佳实践

1. ✅ 使用 CIDR 表示法定义范围
2. ✅ 创建池后立即保留特殊 IP（网关、广播等）
3. ✅ 使用有意义的 subinstance 名称方便管理
4. ✅ 定期检查池统计信息
5. ✅ 批量清理使用 `release_by_subinstance`
