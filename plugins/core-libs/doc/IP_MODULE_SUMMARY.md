# IP Pool Management Module - Summary

## 概述

为 scheduler core-libs 添加了完整的 IP 地址池管理功能，支持 IP 范围管理、分配/释放、以及通过 subinstance/subid/subtype 将 IP 绑定到各种资源类型。

## 新增文件

### 核心代码
- `src/ip/mod.rs` - IP 池管理模块（670+ 行代码）
  - IpPool - IP 池管理器
  - IpRange - IP 范围定义（支持 CIDR）
  - IpBinding - IP 绑定信息
  - ResourceType - 资源类型枚举
  - PoolStats - 池统计信息
  - IpPoolError - 错误类型

### 文档
- `doc/IP_POOL_USAGE.md` - 详细使用指南（300+ 行）
  - API 参考
  - 使用示例
  - 最佳实践
  
- `examples/ip_pool_examples.rs` - 实际代码示例（200+ 行）
  - 5 个完整示例场景
  - 多租户、MAC 绑定、容器管理等

### 更新的文件
- `src/lib.rs` - 添加 ip 模块导出
- `src/README.md` - 添加 ip 模块说明
- `MODULE_STRUCTURE.md` - 更新模块结构文档

## 核心功能

### 1. IP 范围管理
```rust
// CIDR 表示法（推荐）
pool.add_cidr_range("192.168.1.0/24")?;
pool.add_cidr_range("10.0.0.0/16")?;

// 或者指定起止地址
let range = IpRange::new(start_ip, end_ip)?;
pool.add_range(range);
```

### 2. IP 分配
```rust
// 自动分配（池自动选择可用 IP）
let ip = pool.allocate(
    "instance-01",           // subinstance（租户/命名空间）
    "vm-001",                // subid（资源 ID）
    ResourceType::Mac("00:11:22:33:44:55".to_string())
)?;

// 分配特定 IP
pool.allocate_specific(ip, "instance", "id", resource_type)?;
```

### 3. 资源类型支持
- **MAC 地址**: `ResourceType::Mac(String)`
- **虚拟机**: `ResourceType::Vm(String)`
- **容器**: `ResourceType::Container(String)`
- **Pod**: `ResourceType::Pod(String)`
- **自定义**: `ResourceType::Custom(String)`

### 4. 查询功能
```rust
// 通过 subinstance + subid 查找
pool.find_by_subid("instance", "id")?;

// 通过资源类型查找
pool.find_by_resource("mac", "AA:BB:CC:DD:EE:FF")?;

// 列出某个 subinstance 的所有 IP
pool.list_by_subinstance("instance")?;

// 获取绑定详情
pool.get_binding(&ip)?;
```

### 5. IP 释放
```rust
// 按 IP 释放
pool.release_by_ip(&ip)?;

// 按 subinstance + subid 释放
pool.release_by_subid("instance", "id")?;

// 释放整个 subinstance 的所有 IP
pool.release_by_subinstance("instance")?;
```

### 6. 保留 IP
```rust
// 保留网关、广播等特殊地址
pool.reserve_ip(gateway_ip);
pool.reserve_ip(broadcast_ip);

// 取消保留
pool.unreserve_ip(&ip);
```

### 7. 统计信息
```rust
let stats = pool.stats();
println!("Total: {}", stats.total);
println!("Allocated: {}", stats.allocated);
println!("Reserved: {}", stats.reserved);
println!("Available: {}", stats.available);
```

## 使用场景

### 场景 1: 多租户 IP 管理
```rust
// 租户 A
pool.allocate("tenant-a", "vm-01", ResourceType::Vm(...))?;
pool.allocate("tenant-a", "vm-02", ResourceType::Vm(...))?;

// 租户 B
pool.allocate("tenant-b", "pod-01", ResourceType::Pod(...))?;

// 清理租户
pool.release_by_subinstance("tenant-a");
```

### 场景 2: MAC 地址绑定
```rust
// 分配 IP 给 MAC 地址
let ip = pool.allocate(
    "network-segment-1",
    "device-001",
    ResourceType::Mac("AA:BB:CC:DD:EE:FF".to_string())
)?;

// 根据 MAC 查找 IP
let ip = pool.find_by_resource("mac", "AA:BB:CC:DD:EE:FF")?;
```

### 场景 3: Kubernetes Pod 网络
```rust
// Pod 创建时分配 IP
let ip = pool.allocate(
    "namespace-production",
    "nginx-pod-abc123",
    ResourceType::Pod("nginx-pod-abc123".to_string())
)?;

// Pod 删除时释放 IP
pool.release_by_subid("namespace-production", "nginx-pod-abc123")?;
```

### 场景 4: 保留特殊 IP
```rust
pool.add_cidr_range("192.168.1.0/24")?;

// 保留网关和特殊地址
pool.reserve_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));   // 网关
pool.reserve_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 255))); // 广播

// 为关键服务分配特定 IP
pool.allocate_specific(
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
    "infrastructure",
    "load-balancer",
    ResourceType::Vm("lb-primary".to_string())
)?;
```

## 设计特点

### 1. 三层标识体系
- **subinstance**: 实例/租户/命名空间级别
- **subid**: 资源 ID 级别
- **subtype**: 资源类型和具体标识符

这种设计支持：
- 多租户隔离
- 资源分组管理
- 灵活的查询方式
- 批量操作（按 subinstance）

### 2. 幂等性
相同的 (subinstance, subid) 多次分配返回同一个 IP：
```rust
let ip1 = pool.allocate("inst", "id", type1)?;
let ip2 = pool.allocate("inst", "id", type2)?;
assert_eq!(ip1, ip2); // 返回相同 IP
```

### 3. 索引优化
内部维护多个索引以支持快速查询：
- `subinstance_index`: subinstance -> IPs
- `subid_index`: (subinstance, subid) -> IP
- `resource_index`: resource_key -> IP

### 4. IPv4/IPv6 支持
完整支持 IPv4 和 IPv6 地址范围管理。

### 5. 跨平台
- WASM32: 使用占位符时间戳
- Native: 使用系统时间

## 测试覆盖

新增 6 个单元测试：
- ✅ `test_ip_range_creation` - IP 范围创建和验证
- ✅ `test_cidr_range` - CIDR 表示法解析
- ✅ `test_ip_pool_allocation` - IP 分配和幂等性
- ✅ `test_ip_release` - IP 释放
- ✅ `test_find_by_resource` - 资源查找
- ✅ `test_pool_stats` - 统计信息

所有测试通过：21/21 (原有 15 + 新增 6)

## 构建状态

✅ Native 测试通过: `cargo test`
✅ WASM 组件构建成功: `cargo component build --release`
✅ 组件大小: ~474 KB（与之前相同）

## API 总结

### IpPool 方法
| 方法 | 用途 |
|------|------|
| `new(name)` | 创建 IP 池 |
| `add_range(range)` | 添加 IP 范围 |
| `add_cidr_range(cidr)` | 添加 CIDR 范围 |
| `reserve_ip(ip)` | 保留 IP |
| `allocate(...)` | 自动分配 IP |
| `allocate_specific(...)` | 分配特定 IP |
| `release_by_ip(ip)` | 释放 IP |
| `release_by_subid(...)` | 按标识释放 |
| `release_by_subinstance(...)` | 批量释放 |
| `find_by_subid(...)` | 查找 IP |
| `find_by_resource(...)` | 按资源查找 |
| `list_by_subinstance(...)` | 列出所有 IP |
| `get_binding(ip)` | 获取绑定信息 |
| `stats()` | 获取统计 |

### 错误类型
- `InvalidIpAddress` - 无效 IP 地址
- `InvalidRange` - 无效范围
- `IpNotAvailable` - IP 不可用
- `IpNotFound` - IP 未找到
- `IpAlreadyAllocated` - IP 已分配
- `BindingNotFound` - 绑定未找到
- `PoolFull` - 池已满
- `InvalidSubnet` - 无效子网

## 后续建议

1. **持久化**: 添加序列化支持（serde），可保存/恢复池状态
2. **通知机制**: IP 分配/释放事件回调
3. **租约机制**: IP 租期管理，自动回收过期 IP
4. **IP 标签**: 为 IP 添加标签，支持更灵活的查询
5. **池分片**: 大规模场景下的池分片策略
6. **WASI 集成**: 与 WASI 网络接口集成

## 文档完整性

✅ 代码注释完整
✅ API 文档完善
✅ 使用指南详细 (IP_POOL_USAGE.md)
✅ 代码示例丰富 (examples/ip_pool_examples.rs)
✅ 模块文档更新 (MODULE_STRUCTURE.md, README.md)

## 总结

成功为 scheduler core-libs 添加了功能完整、文档齐全的 IP 池管理模块，支持：
- ✅ IP 范围管理（CIDR 支持）
- ✅ IP 分配和释放
- ✅ 多种资源类型绑定（MAC、VM、容器、Pod 等）
- ✅ 三层标识体系（subinstance/subid/subtype）
- ✅ 保留 IP 管理
- ✅ 多索引快速查询
- ✅ 统计和监控
- ✅ IPv4/IPv6 支持
- ✅ 完整测试覆盖
- ✅ WASM 组件兼容
