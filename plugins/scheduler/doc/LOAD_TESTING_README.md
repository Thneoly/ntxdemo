# 负载测试功能 - 快速入门

## 🎯 功能概述

基于 wasm32-wasip2 组件技术的网络性能测试工具，支持：
- ⏱️ **用户上线模型**: 按时间表生成用户（如：第1秒100用户，第2秒30用户）
- 🔄 **用户生命周期**: 支持单次执行或循环执行（可配置迭代次数和间隔）
- 🌐 **IP 池管理**: 每个用户从 IP 池分配源 IP，支持多租户隔离
- 📊 **监控指标**: 实时统计用户、请求、IP 池状态

## 📁 文件结构

```
plugins/scheduler/
├── res/
│   ├── http_scenario.yaml            # 原场景 + 负载配置
│   ├── load_test_simple.yaml         # 简单场景 (10 用户)
│   └── load_test_advanced.yaml       # 高级场景 (500 用户，多租户)
├── doc/
│   ├── LOAD_TESTING_DESIGN.md        # 📖 完整设计文档
│   ├── IMPLEMENTATION_GUIDE.md       # 🔧 实施指南
│   └── LOAD_TESTING_SUMMARY.md       # 📋 功能总结
└── actions-http/
    └── IP_POOL_INTEGRATION.md        # IP 池集成说明
```

## 🚀 快速开始

### 1. 查看示例场景

#### 简单场景 (10 用户)

```bash
cat res/load_test_simple.yaml
```

**配置说明**:
- 第 1 秒上线 10 个用户
- 每个用户执行 2 次任务
- 每次执行间隔 1 秒
- 每个用户独占一个 IP (10.0.1.0 - 10.0.1.9)

#### 高级场景 (500 用户，多租户)

```bash
cat res/load_test_advanced.yaml
```

**配置说明**:
- 5 个阶段，逐步增加用户（0s: 50, 10s: 100, 30s: 30, 45s: 20, 60s: 200, 90s: 100）
- 3 个租户（tenant-a, tenant-b, internal）
- 每个租户使用独立的 IP 池
- 用户执行 5 次迭代，间隔 2 秒

### 2. 配置解析

#### IP 池定义

```yaml
workbook:
  ip_pools:
    - id: eip-pool-1              # IP 池 ID
      name: "HTTP Client EIP Pool"
      ranges:
        - "10.0.1.0/24"           # CIDR 格式，254 个 IP
        - "10.0.2.0/24"           # 再加 254 个 IP
      allocation_strategy: round_robin  # 分配策略
```

#### 用户上线配置

```yaml
load:
  ramp_up:
    phases:
      - at_second: 1              # 时间点（秒）
        spawn_users: 100          # 用户数量
        tenant_id: "tenant-a"     # 租户 ID（可选）
        ip_pool_override: tenant-a-pool  # 覆盖默认 IP 池（可选）
```

#### 用户生命周期

```yaml
load:
  user_lifetime:
    mode: loop                    # loop: 循环, once: 单次
    iterations: 3                 # 循环次数 (0 = 无限)
    think_time: 1s                # 迭代间隔
```

#### IP 资源绑定

```yaml
load:
  user_resources:
    ip_binding:
      enabled: true               # 是否启用
      pool_id: eip-pool-1         # 使用的 IP 池
      strategy: per_user          # per_user, shared, per_task
      release_on: user_exit       # task_end, user_exit
```

**策略对比**:
- `per_user`: 每个用户独占一个 IP，直到退出
- `shared`: 多个用户共享 IP 池（动态分配）
- `per_task`: 每次任务执行时分配新 IP

#### Actions 使用分配的 IP

```yaml
actions:
  - id: http-request
    call: get
    with:
      url: "http://{{resource.ip}}:{{resource.port}}/api"
      bind_ip: "{{user.allocated_ip}}"  # 使用分配的 IP
```

## 📊 执行流程

### 示例: 100 用户 × 3 次迭代

```
t=0s:   Scheduler 初始化
        └─ 创建 IP 池 eip-pool-1 (508 IPs)

t=1s:   生成 100 个用户
        ├─ User-001: IP=10.0.1.0
        ├─ User-002: IP=10.0.1.1
        └─ User-100: IP=10.0.1.99

t=1s-2s: 100 users 执行第 1 次 workflow
        User-001:
          probe-get (bind_ip=10.0.1.0) → status=200
          push-post (bind_ip=10.0.1.0) → status=200

t=2s:   think_time (1s)

t=2s-3s: 100 users 执行第 2 次 workflow

t=3s:   think_time (1s)

t=3s-4s: 100 users 执行第 3 次 workflow

t=4s:   100 users 退出
        └─ 释放所有 IP (508 available)

统计:
  - 总请求: 100 users × 3 iterations × 2 actions = 600
  - IP 池最终状态: 508 available, 0 allocated
```

## 🔧 实施状态

### ✅ 已完成 (可直接使用)

- [x] **IP 池 API** (core-libs)
  - `IpPool::new()`, `add_cidr_range()`, `allocate()`, `release_by_ip()`
  
- [x] **Socket API** (core-libs)
  - `Socket::bind_to_ip()` - 绑定源 IP
  
- [x] **Actions-HTTP** (actions-http)
  - 支持 `bind_ip` 参数
  - 自动绑定源 IP 后发送请求
  
- [x] **YAML 配置设计**
  - 完整的 load 配置结构
  - 三个示例场景文件
  
- [x] **文档**
  - 设计文档 (21KB)
  - 实施指南 (19KB)
  - 功能总结 (13KB)

### ⏳ 待实施 (需要开发)

- [ ] **DSL 数据结构** (scheduler-core)
  - LoadSection, RampUpPhase, UserLifetimeConfig, etc.
  - 扩展 Scenario 支持 load 配置
  
- [ ] **IP 池管理器** (scheduler)
  - IpPoolManager: 初始化、分配、释放
  
- [ ] **用户执行器** (scheduler/executor)
  - UserContext, UserExecutor
  - 变量替换: `{{user.allocated_ip}}`
  
- [ ] **Scheduler 集成** (scheduler)
  - 按时间表生成用户
  - 并发执行管理
  - 统计和监控

## 📖 文档导航

### 🎯 我想了解...

**整体设计和架构**
→ 阅读 [`LOAD_TESTING_DESIGN.md`](LOAD_TESTING_DESIGN.md)
  - 架构组件说明
  - 核心概念定义
  - 执行流程详解
  - 多租户场景

**如何实施代码**
→ 阅读 [`IMPLEMENTATION_GUIDE.md`](IMPLEMENTATION_GUIDE.md)
  - 分步实施指南
  - 代码示例（Rust）
  - 测试步骤
  - 故障排查

**功能特性和进度**
→ 阅读 [`LOAD_TESTING_SUMMARY.md`](LOAD_TESTING_SUMMARY.md)
  - 已完成的功能
  - 待实施的任务
  - 性能目标
  - 团队协作

**IP 池如何集成**
→ 阅读 [`actions-http/IP_POOL_INTEGRATION.md`](../actions-http/IP_POOL_INTEGRATION.md)
  - IP 池 API 使用
  - 源 IP 绑定示例
  - 工作流程

## 🧪 测试计划

### Phase 1: 单元测试

```bash
# 测试 DSL 解析
cargo test -p scheduler-core test_load_section

# 测试 IP 池管理器
cargo test -p scheduler test_ip_manager

# 测试用户执行器
cargo test -p scheduler test_user_executor
```

### Phase 2: 功能测试

```bash
# 简单场景 (10 用户)
cargo run --bin scheduler -- res/load_test_simple.yaml

# 验证:
# - 10 个用户在 t=1s 生成
# - 每个用户分配了唯一的 IP (10.0.1.0 - 10.0.1.9)
# - 每个用户执行 2 次 workflow
# - 所有 IP 最终被释放
```

### Phase 3: 压力测试

```bash
# 高级场景 (500 用户)
cargo run --bin scheduler --release -- res/load_test_advanced.yaml

# 验证:
# - 500 用户按阶段生成
# - 3 个租户使用不同的 IP 段
# - 内存占用 < 500MB
# - CPU 利用率 < 80%
```

## 🎓 使用案例

### 案例 1: 快速健康检查

```yaml
load:
  ramp_up:
    phases:
      - at_second: 1
        spawn_users: 10
  user_lifetime:
    mode: once  # 每个用户执行一次
```

**用途**: 快速验证服务是否正常

### 案例 2: 持续压力测试

```yaml
load:
  ramp_up:
    phases:
      - at_second: 1
        spawn_users: 100
  user_lifetime:
    mode: loop
    iterations: 0  # 无限循环
    think_time: 500ms
```

**用途**: 长期稳定性测试

### 案例 3: 阶梯式负载

```yaml
load:
  ramp_up:
    phases:
      - at_second: 0
        spawn_users: 10      # 预热
      - at_second: 60
        spawn_users: 90      # 1分钟: 100 users
      - at_second: 120
        spawn_users: 100     # 2分钟: 200 users
      - at_second: 180
        spawn_users: 300     # 3分钟: 500 users
```

**用途**: 找到系统容量上限

### 案例 4: 多租户隔离测试

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
      - at_second: 2
        spawn_users: 50
        tenant_id: "tenant-b"
        ip_pool_override: tenant-b-pool
```

**用途**: 验证租户流量隔离

## 🐛 故障排查

### 问题: IP 池耗尽

```
Error: Failed to allocate IP: No available IPs
```

**原因**: 用户数超过 IP 池容量

**解决**:
1. 增加 CIDR 范围: `ranges: ["10.0.1.0/24", "10.0.2.0/24"]`
2. 使用共享策略: `strategy: shared`
3. 减少并发用户数

### 问题: 变量未替换

```
Error: Invalid IP address: {{user.allocated_ip}}
```

**原因**: 变量替换逻辑未实现

**解决**: 参考 [`IMPLEMENTATION_GUIDE.md`](IMPLEMENTATION_GUIDE.md) Step 3 实现变量替换

### 问题: 用户生成延迟

**现象**: 用户实际生成时间晚于配置

**原因**: 系统负载高，线程创建慢

**解决**:
1. 使用 `--release` 编译
2. 使用线程池代替动态创建
3. 增加系统资源

## 🚦 下一步行动

### 对于开发者

1. **阅读实施指南**: [`IMPLEMENTATION_GUIDE.md`](IMPLEMENTATION_GUIDE.md)
2. **实现 DSL 扩展**: 添加 LoadSection 等数据结构
3. **实现 IpPoolManager**: IP 池生命周期管理
4. **实现 UserExecutor**: 用户任务执行逻辑
5. **集成到 Scheduler**: 主循环调用
6. **编写单元测试**: 验证各模块功能
7. **运行集成测试**: 使用示例场景验证

### 对于 QA

1. **熟悉示例场景**: 理解配置含义
2. **准备测试环境**: 确保有足够资源
3. **功能测试**: 使用 `load_test_simple.yaml`
4. **压力测试**: 使用 `load_test_advanced.yaml`
5. **性能测试**: 监控 CPU、内存、网络
6. **报告问题**: 记录错误和性能数据

### 对于产品

1. **审查设计**: [`LOAD_TESTING_DESIGN.md`](LOAD_TESTING_DESIGN.md)
2. **验收标准**: 确认功能和性能要求
3. **用户文档**: 基于本 README 编写用户手册
4. **Demo 准备**: 使用简单场景演示

## 📞 支持

- **技术问题**: 查阅 [`IMPLEMENTATION_GUIDE.md`](IMPLEMENTATION_GUIDE.md) 的故障排查部分
- **设计问题**: 查阅 [`LOAD_TESTING_DESIGN.md`](LOAD_TESTING_DESIGN.md)
- **IP 池问题**: 查阅 [`actions-http/IP_POOL_INTEGRATION.md`](../actions-http/IP_POOL_INTEGRATION.md)

## 📝 更新日志

### 2024-11-30

- ✅ 完成负载测试设计
- ✅ 创建三个示例场景
- ✅ 编写完整文档（设计、实施、总结）
- ✅ 更新原有场景支持负载配置
- ⏳ 等待 Scheduler/Executor 团队实施

---

**版本**: 1.0.0  
**维护者**: Scheduler Team  
**最后更新**: 2024-11-30
