# 负载测试功能实现总结

## 概述

已完成基于 wasm32-wasip2 组件技术的网络性能测试工具的负载配置设计，支持：
- ✅ 可配置的用户上线模型（ramp-up）
- ✅ IP 池资源分配与管理
- ✅ 多租户隔离
- ✅ 灵活的用户生命周期配置

## 完成的工作

### 1. YAML 配置增强

**文件**: `res/http_scenario.yaml`

添加了完整的负载测试配置：
- `workbook.ip_pools`: IP 池定义，支持多个 CIDR 范围
- `load.ramp_up.phases`: 用户上线时间表
- `load.user_lifetime`: 用户执行模式（once/loop）
- `load.user_resources.ip_binding`: IP 绑定策略
- Actions 中使用 `{{user.allocated_ip}}` 变量

### 2. 示例场景文件

创建了三个测试场景：

#### `res/load_test_simple.yaml`
- **场景**: 10 用户基础测试
- **配置**: 第 1 秒上线 10 用户
- **执行**: 每用户循环 2 次
- **用途**: 快速验证功能

#### `res/load_test_advanced.yaml`
- **场景**: 多租户复杂压力测试
- **配置**: 5 个阶段，共 500 用户
- **租户**: Tenant A (350 用户), Tenant B (130 用户), Internal (20 用户)
- **用途**: 完整功能验证和性能测试

#### `res/http_scenario.yaml` (已修改)
- **场景**: 原有三阶段 HTTP 测试
- **增强**: 添加 IP 池配置和 bind_ip 参数
- **兼容**: 保留原有 workflow 逻辑

### 3. 设计文档

#### `doc/LOAD_TESTING_DESIGN.md` (~1500 行)
完整的设计文档，包含：
- 架构组件说明
- 核心概念定义（用户、任务、动作）
- YAML 配置详解
- 实现架构和组件职责
- 执行流程示例
- 多租户场景
- 性能考虑
- 监控指标

#### `doc/IMPLEMENTATION_GUIDE.md` (~800 行)
分步实施指南，包含：
- DSL 数据结构扩展
- IP 池管理器实现
- 用户上下文和执行器
- Scheduler 主循环集成
- 测试步骤和验收标准
- 故障排查指南

## 技术架构

```
┌─────────────┐    ┌──────────────┐    ┌──────────────┐
│  Scheduler  │───▶│   Executor   │───▶│ Actions-HTTP │
│   (调度器)   │    │   (执行器)    │    │  (HTTP动作)   │
└─────────────┘    └──────────────┘    └──────────────┘
       │                   │                    │
       │                   │                    │
       ▼                   ▼                    ▼
┌─────────────────────────────────────────────────────┐
│              Core-Libs (核心库)                       │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────┐  │
│  │   IP Pool    │  │    Socket    │  │  Timer   │  │
│  │  (IP资源池)   │  │  (Socket API)│  │  (定时器) │  │
│  └──────────────┘  └──────────────┘  └──────────┘  │
└─────────────────────────────────────────────────────┘
```

## 核心功能

### 1. 用户上线模型（Ramp-Up）

支持按时间表生成用户：

```yaml
load:
  ramp_up:
    phases:
      - at_second: 1       # 第 1 秒
        spawn_users: 100   # 上线 100 用户
      - at_second: 2       # 第 2 秒
        spawn_users: 30    # 再上线 30 用户
```

### 2. 用户生命周期

支持两种执行模式：

```yaml
load:
  user_lifetime:
    mode: loop           # loop: 循环执行, once: 执行一次
    iterations: 3        # 循环次数 (0 = 无限)
    think_time: 1s       # 迭代间隔
```

### 3. IP 资源池管理

支持三种分配策略：

```yaml
load:
  user_resources:
    ip_binding:
      enabled: true
      pool_id: eip-pool-1
      strategy: per_user   # per_user, shared, per_task
      release_on: user_exit # task_end, user_exit
```

| 策略 | 说明 | 使用场景 |
|------|------|----------|
| `per_user` | 每个用户独占一个 IP | 需要保持连接的场景 |
| `shared` | 多个用户共享 IP 池 | IP 资源受限场景 |
| `per_task` | 每次任务分配新 IP | 需要频繁切换 IP 的场景 |

### 4. 多租户支持

每个租户可以使用独立的 IP 池：

```yaml
workbook:
  ip_pools:
    - id: tenant-a-pool
      ranges: ["10.0.1.0/24"]
    - id: tenant-b-pool
      ranges: ["10.0.2.0/24"]

load:
  ramp_up:
    phases:
      - at_second: 1
        spawn_users: 100
        tenant_id: "tenant-a"
        ip_pool_override: tenant-a-pool
```

### 5. 变量替换

Actions 支持动态变量：

```yaml
actions:
  - id: http-request
    call: get
    with:
      url: "http://{{resource.ip}}:{{resource.port}}/api"
      bind_ip: "{{user.allocated_ip}}"  # 使用分配的 IP
      headers:
        X-User-ID: "{{user.id}}"
        X-Tenant-ID: "{{tenant.id}}"
```

## 执行流程示例

### 场景: 100 用户在第 1 秒上线，每用户执行 3 次任务

```
t=0s:   Scheduler 初始化
        └─ 创建 IP 池 (508 IPs available)

t=1s:   Spawn 100 users
        ├─ User-001: allocate IP 10.0.1.0
        ├─ User-002: allocate IP 10.0.1.1
        └─ User-100: allocate IP 10.0.1.99

t=1s-2s: 执行第 1 次 workflow
t=2s:    think_time (1s)
t=2s-3s: 执行第 2 次 workflow
t=3s:    think_time (1s)
t=3s-4s: 执行第 3 次 workflow

t=4s:   100 users exit
        └─ 释放所有 IP (508 available, 0 allocated)
```

## 实施状态

### ✅ 已完成

- [x] YAML 配置设计
- [x] 示例场景文件（简单、复杂、多租户）
- [x] 完整设计文档
- [x] 分步实施指南
- [x] IP 池 API（core-libs）
- [x] Socket bind_to_ip 支持（core-libs）
- [x] Actions-HTTP bind_ip 参数

### ⏳ 待实施

- [ ] DSL 数据结构扩展（LoadSection, RampUpPhase, etc.）
- [ ] IP 池管理器（IpPoolManager）
- [ ] 用户上下文和执行器（UserContext, UserExecutor）
- [ ] Scheduler 主循环集成
- [ ] 并发控制和速率限制
- [ ] 监控指标导出（Prometheus）

## 性能目标

- **并发用户**: 500 用户（单机）
- **生成速率**: ≥ 100 用户/秒
- **内存占用**: < 500MB (500 用户场景)
- **CPU 利用率**: < 80% (4 核)
- **IP 分配延迟**: < 1ms
- **响应时间 P99**: < 100ms

## 监控指标

### Scheduler 指标
```
scheduler_users_spawned_total{tenant="tenant-a"} 100
scheduler_users_active{tenant="tenant-a"} 95
scheduler_ip_pool_available{pool="eip-pool-1"} 408
```

### Executor 指标
```
executor_tasks_executed_total{action="probe-get"} 300
executor_tasks_success_total{action="probe-get"} 297
executor_tasks_duration_seconds{action="probe-get",quantile="0.99"} 0.045
```

### Actions-HTTP 指标
```
http_requests_total{method="GET",bind_ip="10.0.1.0"} 3
http_requests_duration_seconds{method="GET",quantile="0.99"} 0.035
```

## 使用示例

### 1. 快速测试（10 用户）

```bash
cargo run --bin scheduler -- res/load_test_simple.yaml
```

### 2. 完整压力测试（500 用户）

```bash
cargo run --bin scheduler --release -- res/load_test_advanced.yaml
```

### 3. 自定义场景

```yaml
version: "1.0"
name: my_load_test

workbook:
  ip_pools:
    - id: my-pool
      ranges: ["192.168.1.0/24"]

load:
  ramp_up:
    phases:
      - at_second: 1
        spawn_users: 50
  user_lifetime:
    mode: loop
    iterations: 10
    think_time: 500ms
  user_resources:
    ip_binding:
      enabled: true
      pool_id: my-pool
      strategy: per_user
      release_on: user_exit

actions:
  - id: my-request
    call: get
    with:
      url: "http://example.com/api"
      bind_ip: "{{user.allocated_ip}}"

workflows:
  nodes:
    - id: start
      type: action
      action: my-request
      edges:
        - to: end
    - id: end
      type: end
```

## 故障排查

### 问题 1: IP 池耗尽

```
Error: Failed to allocate IP: No available IPs
```

**解决方案**:
1. 增加 CIDR 范围: `ranges: ["10.0.1.0/24", "10.0.2.0/24"]`
2. 使用共享策略: `strategy: shared`
3. 减少并发用户数

### 问题 2: 用户生成延迟

**症状**: 用户实际生成时间晚于配置时间

**解决方案**:
1. 使用 `--release` 编译
2. 增加 `spawn_rate_limit`
3. 检查系统负载

## 下一步计划

### Phase 1: 核心实现 (Week 1)

1. 扩展 DSL 数据结构
2. 实现 IpPoolManager
3. 实现 UserExecutor
4. 集成到 Scheduler 主循环

### Phase 2: 功能完善 (Week 2)

1. 并发控制和速率限制
2. 多租户 IP 隔离
3. IP 池容量管理
4. 错误处理和重试

### Phase 3: 监控优化 (Week 3)

1. Prometheus 指标导出
2. 实时监控面板
3. 性能优化（异步 I/O）
4. 压力测试报告

## 参考文档

- **设计文档**: [`doc/LOAD_TESTING_DESIGN.md`](LOAD_TESTING_DESIGN.md)
- **实施指南**: [`doc/IMPLEMENTATION_GUIDE.md`](IMPLEMENTATION_GUIDE.md)
- **IP 池集成**: [`actions-http/IP_POOL_INTEGRATION.md`](../actions-http/IP_POOL_INTEGRATION.md)
- **Actions-HTTP 架构**: [`actions-http/ARCHITECTURE.md`](../actions-http/ARCHITECTURE.md)
- **简单场景**: [`res/load_test_simple.yaml`](../res/load_test_simple.yaml)
- **高级场景**: [`res/load_test_advanced.yaml`](../res/load_test_advanced.yaml)

## 文件清单

```
plugins/scheduler/
├── res/
│   ├── http_scenario.yaml            # 已更新：添加 IP 池和 load 配置
│   ├── load_test_simple.yaml         # 新建：10 用户简单测试
│   └── load_test_advanced.yaml       # 新建：500 用户多租户测试
├── doc/
│   ├── LOAD_TESTING_DESIGN.md        # 新建：完整设计文档 (~1500 行)
│   ├── IMPLEMENTATION_GUIDE.md       # 新建：实施指南 (~800 行)
│   └── LOAD_TESTING_SUMMARY.md       # 本文件
└── actions-http/
    ├── IP_POOL_INTEGRATION.md        # 已有：IP 池集成指南
    └── src/component.rs              # 已有：支持 bind_ip 参数
```

## 团队协作

### Scheduler Team (优先级: High)

**任务**:
1. 实现 DSL 扩展（LoadSection, RampUpPhase）
2. 实现 IpPoolManager
3. 实现用户生成和调度逻辑

**参考**: `doc/IMPLEMENTATION_GUIDE.md` Step 1-4

### Executor Team (优先级: High)

**任务**:
1. 实现 UserContext 和 UserExecutor
2. 实现变量替换逻辑（{{user.allocated_ip}}）
3. 集成到 ActionComponent 执行流程

**参考**: `doc/IMPLEMENTATION_GUIDE.md` Step 3

### Core-Libs Team (优先级: Low)

**任务**: 
- ✅ 已完成（IP Pool, Socket API）
- 可选：优化 IP 分配性能

### QA Team (优先级: Medium)

**任务**:
1. 使用 `res/load_test_simple.yaml` 进行功能测试
2. 使用 `res/load_test_advanced.yaml` 进行压力测试
3. 验证性能指标达标

## 联系方式

**技术负责人**: Scheduler Team Lead
**文档维护**: [Your Name]
**最后更新**: 2024-11-30

---

## 附录: 快速参考

### 最小配置

```yaml
load:
  ramp_up:
    phases:
      - at_second: 1
        spawn_users: 10
  user_lifetime:
    mode: once
  user_resources:
    ip_binding:
      enabled: false
```

### 典型配置

```yaml
load:
  ramp_up:
    phases:
      - at_second: 1
        spawn_users: 100
  user_lifetime:
    mode: loop
    iterations: 3
    think_time: 1s
  user_resources:
    ip_binding:
      enabled: true
      pool_id: client-pool
      strategy: per_user
      release_on: user_exit
```

### 高级配置

```yaml
load:
  ramp_up:
    phases:
      - at_second: 0
        spawn_users: 50
        tenant_id: "tenant-a"
        ip_pool_override: tenant-a-pool
      - at_second: 30
        spawn_users: 30
        tenant_id: "tenant-b"
        ip_pool_override: tenant-b-pool
  user_lifetime:
    mode: loop
    iterations: 0  # 无限循环
    think_time: 2s
  user_resources:
    ip_binding:
      enabled: true
      pool_id: default-pool
      strategy: per_user
      release_on: user_exit
  concurrency:
    max_concurrent_users: 500
    spawn_rate_limit: 100/s
```

---

**版本**: 1.0.0  
**状态**: Design Complete, Implementation Pending  
**下次审查**: 2024-12-07
