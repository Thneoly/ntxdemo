# 负载测试设计 - 用户呼叫模型与 IP 池集成

## 概述

本文档描述了基于 wasm32-wasip2 组件技术的网络性能测试工具的负载配置设计，支持：
- 可配置的用户上线模型（ramp-up）
- 每个用户独立执行任务流
- IP 池资源分配与管理
- 多租户隔离

## 架构组件

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

## 核心概念

### 1. 用户（User）

- **定义**: 一个独立的执行单元，模拟一个并发用户
- **生命周期**: spawn → execute task → (iterate) → exit
- **资源**: 每个用户可以分配独占的 IP 地址、连接池等资源
- **隔离**: 不同用户之间的执行状态、资源、数据完全隔离

### 2. 任务（Task）

- **定义**: 由多个 Action 组成的工作流
- **执行**: 用户按照 workflow 定义的顺序执行 actions
- **状态**: 每个用户维护自己的任务执行状态

### 3. 动作（Action）

- **定义**: 单个 HTTP 请求/响应操作
- **类型**: GET, POST, PUT, DELETE, etc.
- **参数**: URL, headers, body, bind_ip (源 IP 绑定)

## YAML 配置详解

### 完整示例

```yaml
version: "1.0"
name: http_tri_phase_demo
workbook:
  # IP 资源池配置
  ip_pools:
    - id: eip-pool-1
      name: "HTTP Client EIP Pool"
      ranges:
        - "10.0.1.0/24"    # 254 IPs
        - "10.0.2.0/24"    # 254 IPs
      allocation_strategy: round_robin
      
  # 目标资源
  resources:
    - id: resource
      type: http_endpoint
      properties:
        ip: "192.168.1.100"
        port: "8080"

# 负载配置
load:
  # 用户增长模式
  ramp_up:
    phases:
      - at_second: 1       # 第 1 秒
        spawn_users: 100   # 上线 100 用户
        
      - at_second: 2       # 第 2 秒
        spawn_users: 30    # 再上线 30 用户
        
      - at_second: 5       # 第 5 秒
        spawn_users: 50    # 再上线 50 用户
        
    # 用户生命周期
    user_lifetime:
      mode: loop           # loop 或 once
      iterations: 3        # 循环次数 (0 = 无限)
      think_time: 1s       # 迭代间隔
      
  # 用户资源绑定
  user_resources:
    ip_binding:
      enabled: true
      pool_id: eip-pool-1
      strategy: per_user   # per_user, shared, per_task
      release_on: task_end # task_end, user_exit

actions:
  actions:
    - id: probe-get
      call: get
      with:
        url: "http://{{resource.ip}}:{{resource.port}}/asset"
        bind_ip: "{{user.allocated_ip}}"  # 绑定用户 IP
      export:
        - type: content
          name: status_code
          
    - id: push-post
      call: post
      with:
        url: "http://{{resource.ip}}:{{resource.port}}/asset"
        bind_ip: "{{user.allocated_ip}}"
        headers:
          content-type: application/json
        body: "{\"data\": \"test\"}"

workflows:
  nodes:
    - id: start
      type: action
      action: probe-get
      edges:
        - to: push-node
          trigger:
            condition: "{{probe-get.status_code == 200}}"
        - to: end
          trigger:
            condition: "true"
            
    - id: push-node
      type: action
      action: push-post
      edges:
        - to: end
          
    - id: end
      type: end
```

### 配置字段说明

#### `load.ramp_up.phases`

定义用户上线时间表：

```yaml
phases:
  - at_second: 1        # 时间点（秒）
    spawn_users: 100    # 上线用户数量
    tenant_id: "A"      # 可选：租户标识
```

**执行逻辑**:
1. Scheduler 在 t=1s 时创建 100 个用户实例
2. 每个用户从 IP 池分配一个 IP 地址
3. 用户开始执行 workflow（从 start 节点开始）
4. 在 t=2s 时，再创建 30 个新用户，重复上述流程

#### `load.user_lifetime`

控制用户执行行为：

| 字段 | 类型 | 说明 |
|------|------|------|
| `mode` | `loop` \| `once` | loop: 循环执行任务; once: 执行一次退出 |
| `iterations` | `number` | loop 模式下的循环次数，0 = 无限循环 |
| `think_time` | `duration` | 每次迭代之间的等待时间 |

**示例执行流程（mode: loop, iterations: 3）**:

```
User-001 Timeline:
t=1s:   spawn
t=1s:   start → probe-get → push-post → end (第1次)
t=2s:   (think_time: 1s)
t=2s:   start → probe-get → push-post → end (第2次)
t=3s:   (think_time: 1s)
t=3s:   start → probe-get → push-post → end (第3次)
t=4s:   exit
```

#### `load.user_resources.ip_binding`

配置 IP 资源分配策略：

| 字段 | 类型 | 说明 |
|------|------|------|
| `enabled` | `boolean` | 是否启用 IP 绑定 |
| `pool_id` | `string` | IP 池 ID（引用 `workbook.ip_pools[].id`） |
| `strategy` | `per_user` \| `shared` \| `per_task` | 分配策略 |
| `release_on` | `task_end` \| `user_exit` | 释放时机 |

**策略对比**:

| Strategy | 说明 | 使用场景 |
|----------|------|----------|
| `per_user` | 每个用户独占一个 IP，直到退出 | 需要保持连接、会话的场景 |
| `shared` | 多个用户共享 IP 池（动态分配） | IP 资源受限，短连接场景 |
| `per_task` | 每次任务执行时分配新 IP | 需要频繁切换源 IP 的场景 |

**释放时机**:

| Release On | 说明 | 适用策略 |
|------------|------|----------|
| `task_end` | 每次任务结束后释放 IP | `per_task`, `shared` |
| `user_exit` | 用户退出后释放 IP | `per_user` |

## 实现架构

### 组件职责划分

#### 1. Scheduler (调度器)

**职责**:
- 解析 YAML 配置
- 管理 IP 池创建和初始化
- 按照 ramp_up 配置生成用户
- 调度用户执行任务

**关键逻辑**:

```rust
// 伪代码示例
struct Scheduler {
    ip_pools: HashMap<String, IpPool>,
    users: Vec<User>,
    config: LoadConfig,
}

impl Scheduler {
    fn initialize(&mut self) {
        // 1. 创建 IP 池
        for pool_config in &self.config.ip_pools {
            let mut pool = IpPool::new(&pool_config.id);
            for range in &pool_config.ranges {
                pool.add_cidr_range(range)?;
            }
            self.ip_pools.insert(pool_config.id.clone(), pool);
        }
    }
    
    fn run(&mut self) {
        let start_time = Instant::now();
        
        // 2. 按时间表生成用户
        for phase in &self.config.ramp_up.phases {
            let target_time = start_time + Duration::from_secs(phase.at_second);
            
            // 等待到指定时间点
            sleep_until(target_time);
            
            // 生成用户
            for i in 0..phase.spawn_users {
                let user = self.spawn_user(phase)?;
                self.users.push(user);
            }
        }
        
        // 3. 等待所有用户完成
        for user in &mut self.users {
            user.join()?;
        }
    }
    
    fn spawn_user(&mut self, phase: &RampUpPhase) -> Result<User> {
        // 分配 IP
        let ip = if self.config.user_resources.ip_binding.enabled {
            let pool_id = &self.config.user_resources.ip_binding.pool_id;
            let pool = self.ip_pools.get_mut(pool_id)?;
            
            Some(pool.allocate(
                &phase.tenant_id.unwrap_or("default"),
                &format!("user-{}", self.users.len()),
                ResourceType::Custom("http-client".into()),
            )?)
        } else {
            None
        };
        
        // 创建用户上下文
        let user = User {
            id: self.users.len(),
            allocated_ip: ip,
            workflow: self.config.workflows.clone(),
            actions: self.config.actions.clone(),
            lifetime: self.config.user_lifetime.clone(),
        };
        
        // 启动用户执行线程/任务
        user.start();
        
        Ok(user)
    }
}
```

#### 2. Executor (执行器)

**职责**:
- 执行用户的工作流
- 管理用户状态和资源
- 调用 ActionComponent 执行 actions
- 处理变量替换（如 `{{user.allocated_ip}}`）

**关键逻辑**:

```rust
struct User {
    id: usize,
    allocated_ip: Option<IpAddr>,
    workflow: WorkflowSection,
    actions: ActionsSection,
    lifetime: UserLifetime,
}

impl User {
    fn run(&mut self) -> Result<()> {
        let iterations = match self.lifetime.mode {
            LifetimeMode::Once => 1,
            LifetimeMode::Loop => self.lifetime.iterations,
        };
        
        for iteration in 0..iterations {
            if iteration > 0 {
                sleep(self.lifetime.think_time);
            }
            
            // 执行一次完整的 workflow
            self.execute_workflow()?;
        }
        
        // 释放资源
        if self.config.ip_binding.release_on == ReleaseOn::UserExit {
            self.release_ip()?;
        }
        
        Ok(())
    }
    
    fn execute_workflow(&mut self) -> Result<()> {
        let mut current_node = "start";
        let mut context = ExecutionContext::new();
        
        // 注入用户变量
        context.set("user.id", self.id);
        if let Some(ip) = self.allocated_ip {
            context.set("user.allocated_ip", ip.to_string());
        }
        
        loop {
            let node = self.workflow.find_node(current_node)?;
            
            match node.node_type {
                WorkflowNodeType::Action => {
                    let action = self.actions.find_action(&node.action)?;
                    
                    // 变量替换
                    let resolved_action = self.resolve_variables(action, &context)?;
                    
                    // 执行 action
                    let result = self.execute_action(resolved_action)?;
                    
                    // 保存结果到 context
                    context.merge(result.exports);
                    
                    // 选择下一个节点
                    current_node = self.select_next_node(&node.edges, &context)?;
                    
                    // 释放 IP（如果配置为 task_end）
                    if self.config.ip_binding.release_on == ReleaseOn::TaskEnd {
                        self.release_and_reallocate_ip()?;
                    }
                }
                WorkflowNodeType::End => {
                    break;
                }
            }
        }
        
        Ok(())
    }
    
    fn resolve_variables(&self, action: &ActionDef, context: &ExecutionContext) -> Result<ActionDef> {
        let mut resolved = action.clone();
        
        // 替换 with 参数中的变量
        for (key, value) in &mut resolved.with {
            if let Some(var_value) = value.as_str() {
                // 替换 {{user.allocated_ip}} 等
                *value = serde_json::Value::String(
                    context.interpolate(var_value)?
                );
            }
        }
        
        Ok(resolved)
    }
}
```

#### 3. Actions-HTTP (HTTP 组件)

**职责**:
- 执行 HTTP 请求
- 使用 `bind_ip` 参数绑定源 IP
- 返回响应结果

**已实现** (参考 `actions-http/src/component.rs`):

```rust
pub fn do_http_action(action: ActionDef) -> Result<String, String> {
    // 1. 提取 bind_ip 参数
    let bind_ip = action
        .with
        .get("bind_ip")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<IpAddr>().ok());
    
    // 2. 创建 socket
    let mut socket = Socket::tcp_v4()
        .map_err(|e| format!("Failed to create socket: {:?}", e))?;
    
    // 3. 绑定源 IP（如果指定）
    if let Some(source_ip) = bind_ip {
        socket.bind_to_ip(source_ip, 0)
            .map_err(|e| format!("Failed to bind to IP {}: {:?}", source_ip, e))?;
    }
    
    // 4. 连接目标服务器
    socket.connect(destination_addr)?;
    
    // 5. 发送 HTTP 请求
    let request = HttpRequest::new(method, url)
        .headers(headers)
        .body(body);
    
    socket.send(request.to_bytes())?;
    
    // 6. 接收响应
    let response_bytes = socket.recv(8192)?;
    let response = HttpResponse::parse(&response_bytes)?;
    
    // 7. 返回结果
    Ok(format!("status={} body_len={} from_ip={:?}", 
               response.status_code, 
               response.body.len(),
               bind_ip))
}
```

#### 4. Core-Libs (核心库)

**职责**:
- 提供 IP 池管理 API
- 提供 Socket API (支持 bind_to_ip)
- 提供定时器等其他基础功能

**IP Pool API** (已实现):

```rust
pub struct IpPool {
    pool_id: String,
    available: Vec<IpAddr>,
    allocated: HashMap<IpAddr, AllocationInfo>,
}

impl IpPool {
    pub fn new(pool_id: &str) -> Self;
    pub fn add_cidr_range(&mut self, cidr: &str) -> Result<()>;
    pub fn allocate(&mut self, tenant: &str, resource: &str, resource_type: ResourceType) -> Result<IpAddr>;
    pub fn release_by_ip(&mut self, ip: IpAddr) -> Result<()>;
    pub fn stats(&self) -> IpPoolStats;
}
```

**Socket API** (已实现):

```rust
pub struct Socket {
    // ...
}

impl Socket {
    pub fn tcp_v4() -> Result<Self>;
    pub fn bind_to_ip(&mut self, ip: IpAddr, port: u16) -> Result<()>;
    pub fn connect(&mut self, addr: SocketAddress) -> Result<()>;
    pub fn send(&mut self, data: &[u8]) -> Result<usize>;
    pub fn recv(&mut self, buf: &mut [u8]) -> Result<usize>;
}
```

## 执行流程示例

### 场景: 100 用户在第 1 秒上线，每用户执行 3 次任务

**配置**:
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
      pool_id: eip-pool-1
      strategy: per_user
      release_on: user_exit
```

**时间线**:

```
t=0s:   Scheduler 初始化
        ├─ 创建 IP 池 eip-pool-1 (508 IPs available)
        └─ 加载 workflows 和 actions

t=1s:   Spawn 100 users
        ├─ User-001: allocate IP 10.0.1.0
        │   └─ start workflow (iteration 1/3)
        ├─ User-002: allocate IP 10.0.1.1
        │   └─ start workflow (iteration 1/3)
        ├─ ...
        └─ User-100: allocate IP 10.0.1.99
            └─ start workflow (iteration 1/3)

t=1s-2s: 100 users 执行第 1 次 workflow
        User-001:
          ├─ probe-get (bind_ip=10.0.1.0) → status=200
          ├─ push-post (bind_ip=10.0.1.0) → status=200
          └─ end
        ... (other users executing in parallel)

t=2s:   100 users think_time (1s)

t=2s-3s: 100 users 执行第 2 次 workflow

t=3s:   100 users think_time (1s)

t=3s-4s: 100 users 执行第 3 次 workflow

t=4s:   100 users exit
        ├─ User-001: release IP 10.0.1.0
        ├─ User-002: release IP 10.0.1.1
        └─ ...

t=4s:   Scheduler 完成，输出统计
        ├─ Total requests: 100 users × 3 iterations × 2 actions = 600
        ├─ Success rate: 99.8%
        └─ IP pool final state: 508 available, 0 allocated
```

## 多租户场景

### 配置示例

```yaml
workbook:
  ip_pools:
    - id: tenant-a-pool
      name: "Tenant A IP Pool"
      ranges:
        - "10.0.1.0/24"
        
    - id: tenant-b-pool
      name: "Tenant B IP Pool"
      ranges:
        - "10.0.2.0/24"

load:
  ramp_up:
    phases:
      - at_second: 1
        spawn_users: 100
        tenant_id: "tenant-a"
        ip_pool_override: tenant-a-pool  # 覆盖默认 pool
        
      - at_second: 2
        spawn_users: 50
        tenant_id: "tenant-b"
        ip_pool_override: tenant-b-pool
```

**效果**:
- Tenant A 的 100 个用户使用 10.0.1.x 段 IP
- Tenant B 的 50 个用户使用 10.0.2.x 段 IP
- 租户之间的流量完全隔离

## 性能考虑

### 1. 并发控制

**问题**: 大量用户同时执行可能导致资源竞争

**解决方案**:
```yaml
load:
  concurrency:
    max_concurrent_users: 500  # 最大并发用户数
    spawn_rate_limit: 100/s    # 用户创建速率限制
```

### 2. IP 池容量

**问题**: IP 池容量不足

**解决方案**:
1. 使用 `shared` 或 `per_task` 策略降低 IP 占用
2. 增加 CIDR 范围
3. 配置等待队列：
```yaml
load:
  user_resources:
    ip_binding:
      wait_on_exhaustion: true  # IP 耗尽时等待
      wait_timeout: 30s          # 等待超时
```

### 3. 内存优化

**问题**: 大量用户的状态数据占用内存

**解决方案**:
- 使用稀疏存储（sparse storage）
- 只保留活跃用户的完整状态
- 使用流式聚合避免保存所有结果

## 监控指标

### Scheduler 指标

```
scheduler_users_spawned_total{tenant="tenant-a"} 100
scheduler_users_active{tenant="tenant-a"} 95
scheduler_users_completed_total{tenant="tenant-a"} 5

scheduler_ip_pool_available{pool="eip-pool-1"} 408
scheduler_ip_pool_allocated{pool="eip-pool-1"} 100
```

### Executor 指标

```
executor_tasks_executed_total{user="user-001",action="probe-get"} 3
executor_tasks_success_total{user="user-001",action="probe-get"} 3
executor_tasks_duration_seconds{action="probe-get",quantile="0.99"} 0.045
```

### Actions-HTTP 指标

```
http_requests_total{method="GET",bind_ip="10.0.1.0"} 3
http_requests_duration_seconds{method="GET",quantile="0.99"} 0.035
http_requests_bind_failures_total{reason="address_in_use"} 0
```

## 实施路线图

### Phase 1: 基础功能 (MVP)

- [x] IP 池管理 API (core-libs)
- [x] Socket bind_to_ip 支持 (core-libs)
- [x] Actions-HTTP 支持 bind_ip 参数
- [ ] Scheduler 解析 load 配置
- [ ] Scheduler 用户生成逻辑
- [ ] Executor 用户执行逻辑

### Phase 2: 高级功能

- [ ] 多租户 IP 隔离
- [ ] IP 池容量管理和等待队列
- [ ] 并发控制和速率限制
- [ ] 用户状态持久化

### Phase 3: 监控和优化

- [ ] Prometheus 指标导出
- [ ] 性能分析和优化
- [ ] 稀疏存储实现
- [ ] 自动扩缩容支持

## 参考资料

- [IP Pool Integration Guide](../actions-http/IP_POOL_INTEGRATION.md)
- [Actions-HTTP Architecture](../actions-http/ARCHITECTURE.md)
- [Core-Libs API Documentation](../core-libs/README.md)
- [Runner MVP Design](../../runner/docs/plan/mvp/design.md)

## 常见问题

### Q: 如何测试单个用户的行为？

```yaml
load:
  ramp_up:
    phases:
      - at_second: 1
        spawn_users: 1
  user_lifetime:
    mode: once
```

### Q: 如何实现阶梯式压力测试？

```yaml
load:
  ramp_up:
    phases:
      - at_second: 0
        spawn_users: 10      # 预热
      - at_second: 60
        spawn_users: 90      # 第1分钟结束: 100 users
      - at_second: 120
        spawn_users: 100     # 第2分钟结束: 200 users
      - at_second: 180
        spawn_users: 300     # 第3分钟结束: 500 users
```

### Q: 如何模拟真实用户行为（随机 think time）？

```yaml
load:
  user_lifetime:
    mode: loop
    iterations: 0  # 无限循环
    think_time:
      type: random
      min: 1s
      max: 5s
      distribution: normal  # 正态分布
```

### Q: IP 池如何支持动态扩容？

```rust
// 运行时动态添加 IP 段
scheduler.add_ip_range("eip-pool-1", "10.0.3.0/24")?;
```

---

**最后更新**: 2024-11-30
**版本**: 1.0.0
**维护者**: Scheduler Team
