# 实施指南 - 用户呼叫模型与 IP 池集成

## 快速开始

本文档提供了实施用户呼叫模型（User Ramp-Up）和 IP 池集成的分步指南。

## 前置条件

✅ 已完成的组件:
- **Core-Libs**: IP Pool 和 Socket API
- **Actions-HTTP**: bind_ip 参数支持
- **Executor**: ActionComponent 接口

⏳ 需要实施的功能:
- **Scheduler**: 解析 load 配置，生成用户，调度执行
- **Executor**: 用户上下文管理，变量替换

## 实施步骤

### Step 1: 扩展 DSL 数据结构

**文件**: `core-libs/src/dsl/mod.rs`

```rust
// 在 Scenario 结构中添加 load 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub version: String,
    pub name: String,
    pub workbook: WorkbookSection,
    pub actions: ActionsSection,
    pub workflows: WorkflowSection,
    #[serde(default)]
    pub load: Option<LoadSection>,  // 新增
}

// 定义 load 配置结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadSection {
    pub ramp_up: RampUpConfig,
    pub user_lifetime: UserLifetimeConfig,
    pub user_resources: UserResourcesConfig,
    #[serde(default)]
    pub concurrency: Option<ConcurrencyConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RampUpConfig {
    pub phases: Vec<RampUpPhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RampUpPhase {
    pub at_second: u64,          // 时间点（秒）
    pub spawn_users: usize,      // 用户数量
    #[serde(default)]
    pub tenant_id: Option<String>,         // 租户 ID
    #[serde(default)]
    pub ip_pool_override: Option<String>,  // 覆盖默认 IP 池
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserLifetimeConfig {
    pub mode: UserLifetimeMode,
    pub iterations: usize,       // 0 = 无限循环
    pub think_time: String,      // 例如 "1s", "500ms"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserLifetimeMode {
    Once,
    Loop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserResourcesConfig {
    pub ip_binding: IpBindingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpBindingConfig {
    pub enabled: bool,
    pub pool_id: String,
    pub strategy: IpBindingStrategy,
    pub release_on: ReleaseOn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpBindingStrategy {
    PerUser,
    Shared,
    PerTask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseOn {
    TaskEnd,
    UserExit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrencyConfig {
    pub max_concurrent_users: usize,
    pub spawn_rate_limit: String,  // 例如 "100/s"
}
```

**同时扩展 WorkbookSection 支持 IP 池定义**:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkbookSection {
    #[serde(default)]
    pub resources: IndexMap<String, ResourceDef>,
    #[serde(default)]
    pub metrics: IndexMap<String, MetricDef>,
    #[serde(default)]
    pub ip_pools: Vec<IpPoolDef>,  // 新增
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpPoolDef {
    pub id: String,
    pub name: String,
    pub ranges: Vec<String>,  // CIDR 格式，例如 "10.0.1.0/24"
    #[serde(default)]
    pub allocation_strategy: Option<String>,
}
```

### Step 2: 实现 IP 池管理器

**文件**: `scheduler/src/ip_manager.rs` (新建)

```rust
use std::collections::HashMap;
use std::net::IpAddr;
use scheduler_core::{IpPool, IpPoolDef, ResourceType};
use anyhow::{Context, Result};

pub struct IpPoolManager {
    pools: HashMap<String, IpPool>,
}

impl IpPoolManager {
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
        }
    }
    
    /// 从配置初始化 IP 池
    pub fn initialize_from_config(&mut self, pool_defs: &[IpPoolDef]) -> Result<()> {
        for def in pool_defs {
            let mut pool = IpPool::new(&def.id);
            
            for cidr in &def.ranges {
                pool.add_cidr_range(cidr)
                    .with_context(|| format!("Failed to add CIDR range {} to pool {}", cidr, def.id))?;
            }
            
            self.pools.insert(def.id.clone(), pool);
            println!("✓ Initialized IP pool '{}' with {} ranges", def.id, def.ranges.len());
        }
        
        Ok(())
    }
    
    /// 为用户分配 IP
    pub fn allocate_ip(
        &mut self,
        pool_id: &str,
        tenant_id: &str,
        user_id: &str,
    ) -> Result<IpAddr> {
        let pool = self.pools.get_mut(pool_id)
            .with_context(|| format!("IP pool '{}' not found", pool_id))?;
        
        pool.allocate(
            tenant_id,
            user_id,
            ResourceType::Custom("http-client".into()),
        )
    }
    
    /// 释放 IP
    pub fn release_ip(&mut self, pool_id: &str, ip: IpAddr) -> Result<()> {
        let pool = self.pools.get_mut(pool_id)
            .with_context(|| format!("IP pool '{}' not found", pool_id))?;
        
        pool.release_by_ip(ip)
    }
    
    /// 获取池统计信息
    pub fn get_stats(&self, pool_id: &str) -> Option<String> {
        self.pools.get(pool_id).map(|pool| {
            let stats = pool.stats();
            format!("Pool '{}': {} allocated, {} available", 
                    pool_id, stats.allocated, stats.available)
        })
    }
}
```

### Step 3: 实现用户上下文

**文件**: `scheduler/src/user.rs` (新建)

```rust
use std::net::IpAddr;
use std::time::{Duration, Instant};
use scheduler_core::{ActionDef, ActionsSection, WorkflowSection};
use scheduler_executor::ActionComponent;
use anyhow::Result;

pub struct UserContext {
    pub id: usize,
    pub tenant_id: String,
    pub allocated_ip: Option<IpAddr>,
    pub created_at: Instant,
}

impl UserContext {
    pub fn new(id: usize, tenant_id: String, allocated_ip: Option<IpAddr>) -> Self {
        Self {
            id,
            tenant_id,
            allocated_ip,
            created_at: Instant::now(),
        }
    }
}

pub struct UserExecutor {
    context: UserContext,
    workflow: WorkflowSection,
    actions: ActionsSection,
    iterations: usize,
    think_time: Duration,
}

impl UserExecutor {
    pub fn new(
        context: UserContext,
        workflow: WorkflowSection,
        actions: ActionsSection,
        iterations: usize,
        think_time: Duration,
    ) -> Self {
        Self {
            context,
            workflow,
            actions,
            iterations,
            think_time,
        }
    }
    
    /// 执行用户的所有迭代
    pub fn run<C: ActionComponent>(&mut self, component: &mut C) -> Result<Vec<ExecutionTrace>> {
        let mut traces = Vec::new();
        
        for iteration in 0..self.iterations {
            if iteration > 0 {
                std::thread::sleep(self.think_time);
            }
            
            println!("[User-{}] Starting iteration {}/{}", 
                     self.context.id, iteration + 1, self.iterations);
            
            // 执行一次完整的 workflow
            let mut iteration_traces = self.execute_workflow(component)?;
            traces.append(&mut iteration_traces);
        }
        
        println!("[User-{}] Completed all {} iterations", 
                 self.context.id, self.iterations);
        
        Ok(traces)
    }
    
    /// 执行一次完整的 workflow
    fn execute_workflow<C: ActionComponent>(&mut self, component: &mut C) -> Result<Vec<ExecutionTrace>> {
        // TODO: 实现完整的 workflow 执行逻辑
        // 1. 遍历 workflow 节点
        // 2. 执行 actions
        // 3. 替换变量（{{user.allocated_ip}} 等）
        // 4. 根据条件选择下一个节点
        // 5. 记录执行轨迹
        
        Ok(Vec::new())
    }
    
    /// 替换 action 中的用户变量
    fn resolve_action_variables(&self, action: &ActionDef) -> ActionDef {
        let mut resolved = action.clone();
        
        // 替换 with 参数中的变量
        for (key, value) in &mut resolved.with {
            if let Some(str_val) = value.as_str() {
                let replaced = str_val
                    .replace("{{user.id}}", &self.context.id.to_string())
                    .replace("{{tenant.id}}", &self.context.tenant_id)
                    .replace("{{user.allocated_ip}}", 
                             &self.context.allocated_ip
                                 .map(|ip| ip.to_string())
                                 .unwrap_or_default());
                
                *value = serde_json::Value::String(replaced);
            }
        }
        
        resolved
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionTrace {
    pub user_id: usize,
    pub iteration: usize,
    pub action_id: String,
    pub status: String,
    pub duration_ms: u64,
}
```

### Step 4: 扩展 Scheduler 主循环

**文件**: `scheduler/src/main.rs`

```rust
mod ip_manager;
mod user;

use ip_manager::IpPoolManager;
use user::{UserContext, UserExecutor};
use std::thread;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    // 加载 scenario
    let scenario_path = env::args().nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| /* default path */);
    
    let raw = std::fs::read_to_string(&scenario_path)?;
    let scenario = Scenario::from_yaml_str(&raw)?;
    
    // 检查是否有 load 配置
    if let Some(load_config) = &scenario.load {
        println!("=== Load Testing Mode ===");
        run_load_test(&scenario, load_config)?;
    } else {
        println!("=== Single Execution Mode ===");
        run_single_execution(&scenario)?;
    }
    
    Ok(())
}

fn run_load_test(scenario: &Scenario, load_config: &LoadSection) -> Result<()> {
    // 1. 初始化 IP 池管理器
    let mut ip_manager = IpPoolManager::new();
    ip_manager.initialize_from_config(&scenario.workbook.ip_pools)?;
    
    // 2. 准备用户生成器
    let start_time = Instant::now();
    let mut user_handles = Vec::new();
    let mut user_id_counter = 0;
    
    // 3. 按照 ramp_up 配置生成用户
    for phase in &load_config.ramp_up.phases {
        // 等待到指定时间点
        let target_time = start_time + Duration::from_secs(phase.at_second);
        let now = Instant::now();
        if target_time > now {
            thread::sleep(target_time - now);
        }
        
        println!("\n[t={}s] Spawning {} users (tenant: {})", 
                 phase.at_second, 
                 phase.spawn_users,
                 phase.tenant_id.as_deref().unwrap_or("default"));
        
        // 生成用户
        for i in 0..phase.spawn_users {
            user_id_counter += 1;
            
            // 分配 IP
            let pool_id = phase.ip_pool_override.as_deref()
                .or(Some(&load_config.user_resources.ip_binding.pool_id))
                .unwrap();
            
            let tenant_id = phase.tenant_id.clone()
                .unwrap_or_else(|| "default".to_string());
            
            let allocated_ip = if load_config.user_resources.ip_binding.enabled {
                match ip_manager.allocate_ip(pool_id, &tenant_id, &format!("user-{}", user_id_counter)) {
                    Ok(ip) => {
                        println!("  [User-{}] Allocated IP: {}", user_id_counter, ip);
                        Some(ip)
                    }
                    Err(e) => {
                        eprintln!("  [User-{}] Failed to allocate IP: {}", user_id_counter, e);
                        None
                    }
                }
            } else {
                None
            };
            
            // 创建用户上下文
            let user_context = UserContext::new(
                user_id_counter,
                tenant_id.clone(),
                allocated_ip,
            );
            
            // 创建用户执行器
            let mut user_executor = UserExecutor::new(
                user_context,
                scenario.workflows.clone(),
                scenario.actions.clone(),
                match load_config.user_lifetime.mode {
                    UserLifetimeMode::Once => 1,
                    UserLifetimeMode::Loop => load_config.user_lifetime.iterations,
                },
                parse_duration(&load_config.user_lifetime.think_time)?,
            );
            
            // 在新线程中执行用户任务
            let handle = thread::spawn(move || {
                // TODO: 传入 ActionComponent
                // let mut component = HttpActionComponent::new();
                // user_executor.run(&mut component)
                Ok::<_, anyhow::Error>(Vec::new())
            });
            
            user_handles.push((user_id_counter, allocated_ip, pool_id.to_string(), handle));
        }
    }
    
    // 4. 等待所有用户完成
    println!("\n=== Waiting for all users to complete ===");
    for (user_id, ip, pool_id, handle) in user_handles {
        match handle.join() {
            Ok(Ok(traces)) => {
                println!("[User-{}] Completed with {} traces", user_id, traces.len());
                
                // 释放 IP
                if let Some(ip_addr) = ip {
                    if load_config.user_resources.ip_binding.release_on == ReleaseOn::UserExit {
                        if let Err(e) = ip_manager.release_ip(&pool_id, ip_addr) {
                            eprintln!("[User-{}] Failed to release IP {}: {}", user_id, ip_addr, e);
                        } else {
                            println!("[User-{}] Released IP {}", user_id, ip_addr);
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("[User-{}] Execution failed: {}", user_id, e);
            }
            Err(e) => {
                eprintln!("[User-{}] Thread panicked: {:?}", user_id, e);
            }
        }
    }
    
    // 5. 打印最终统计
    println!("\n=== Load Test Complete ===");
    for pool_id in scenario.workbook.ip_pools.iter().map(|p| &p.id) {
        if let Some(stats) = ip_manager.get_stats(pool_id) {
            println!("{}", stats);
        }
    }
    
    Ok(())
}

fn run_single_execution(scenario: &Scenario) -> Result<()> {
    // 原有的单次执行逻辑
    // ...
    Ok(())
}

fn parse_duration(s: &str) -> Result<Duration> {
    // 简单解析 "1s", "500ms" 等格式
    if let Some(ms) = s.strip_suffix("ms") {
        Ok(Duration::from_millis(ms.parse()?))
    } else if let Some(s_str) = s.strip_suffix("s") {
        Ok(Duration::from_secs(s_str.parse()?))
    } else {
        anyhow::bail!("Invalid duration format: {}", s)
    }
}
```

### Step 5: 集成到 SchedulerPipeline

**文件**: `scheduler/src/engine.rs`

在现有的 `SchedulerPipeline` 中添加负载测试支持：

```rust
impl SchedulerPipeline {
    /// 运行负载测试
    pub fn run_load_test<C: ActionComponent>(
        &mut self,
        component: &mut C,
        load_config: &LoadSection,
    ) -> Result<LoadTestReport, SchedulerError> {
        // 实现负载测试逻辑
        // 1. 初始化 IP 池
        // 2. 生成用户
        // 3. 并发执行
        // 4. 收集统计
        
        todo!("Implement load testing in SchedulerPipeline")
    }
}

pub struct LoadTestReport {
    pub total_users: usize,
    pub total_requests: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub duration_secs: f64,
    pub requests_per_second: f64,
}
```

## 测试步骤

### 1. 单元测试

```bash
# 测试 DSL 解析
cargo test -p scheduler-core test_load_section_parsing

# 测试 IP 池管理器
cargo test -p scheduler test_ip_manager

# 测试用户执行器
cargo test -p scheduler test_user_executor
```

### 2. 简单负载测试

```bash
# 运行简单场景（10 用户）
cargo run --bin scheduler -- res/load_test_simple.yaml
```

预期输出：
```
=== Load Testing Mode ===
✓ Initialized IP pool 'client-pool' with 1 ranges

[t=1s] Spawning 10 users (tenant: default)
  [User-1] Allocated IP: 10.0.1.0
  [User-2] Allocated IP: 10.0.1.1
  ...
  [User-10] Allocated IP: 10.0.1.9

[User-1] Starting iteration 1/2
[User-2] Starting iteration 1/2
...

=== Load Test Complete ===
Pool 'client-pool': 0 allocated, 254 available
```

### 3. 高级负载测试

```bash
# 运行多租户场景（500 用户）
cargo run --bin scheduler --release -- res/load_test_advanced.yaml
```

### 4. 性能验证

```bash
# 使用 perf 分析
perf record cargo run --bin scheduler --release -- res/load_test_advanced.yaml
perf report

# 内存分析
valgrind --tool=massif cargo run --bin scheduler -- res/load_test_simple.yaml
```

## 验收标准

### 功能验收

- [ ] 能够解析 load 配置（ramp_up, user_lifetime, user_resources）
- [ ] 能够按时间表生成用户（精度 ±100ms）
- [ ] 用户能够正确分配和使用 IP 地址
- [ ] 用户能够执行多次迭代（loop 模式）
- [ ] 支持 per_user 和 per_task IP 分配策略
- [ ] IP 能够正确释放（task_end 和 user_exit）
- [ ] 支持多租户 IP 隔离

### 性能验收

- [ ] 能够支持 500 并发用户（单机）
- [ ] 用户生成速率 ≥ 100/s
- [ ] 内存占用 < 500MB（500 用户场景）
- [ ] CPU 利用率 < 80%（4 核场景）
- [ ] IP 分配延迟 < 1ms
- [ ] IP 释放延迟 < 1ms

### 监控验收

- [ ] 能够输出用户统计（spawn, active, completed）
- [ ] 能够输出 IP 池统计（allocated, available）
- [ ] 能够输出动作统计（success, failure, duration）
- [ ] 支持导出 Prometheus 指标

## 故障排查

### 问题 1: IP 池耗尽

**现象**: `Failed to allocate IP: No available IPs`

**解决**:
1. 增加 CIDR 范围
2. 使用 `shared` 或 `per_task` 策略
3. 减少并发用户数
4. 检查 IP 是否正确释放

### 问题 2: 用户生成延迟

**现象**: 用户实际生成时间晚于配置时间

**解决**:
1. 检查系统负载
2. 使用 `--release` 编译
3. 增加 `spawn_rate_limit`
4. 使用线程池代替动态创建线程

### 问题 3: 内存占用过高

**现象**: 内存占用 > 1GB

**解决**:
1. 实现流式聚合（不保存所有 traces）
2. 使用稀疏存储
3. 定期清理已完成用户的状态
4. 使用对象池复用用户上下文

## 下一步

1. **完成 Executor 集成**: 在 UserExecutor 中完整实现 workflow 执行逻辑
2. **添加监控**: 集成 Prometheus 指标导出
3. **性能优化**: 使用异步 I/O 代替多线程
4. **Web UI**: 创建实时监控面板
5. **分布式支持**: 支持多机分布式负载测试

## 参考资料

- [负载测试设计文档](LOAD_TESTING_DESIGN.md)
- [IP 池集成指南](../actions-http/IP_POOL_INTEGRATION.md)
- [Scheduler Engine 源码](../scheduler/src/engine.rs)
- [Runner MVP 文档](../../runner/docs/plan/mvp/mvp.md)

---

**最后更新**: 2024-11-30
**维护者**: Scheduler Team
