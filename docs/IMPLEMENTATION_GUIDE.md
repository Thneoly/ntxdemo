# Echo 场景 Phase 1 实现指南

## 目标

本指南逐步实现：
- **Echo Server Wasm 组件**：Host-1 上接收 UDP 包，调用 Wasm on_packet()，返回 echo 响应
- **Echo Client Wasm 组件**：Host-2 上生成请求，验证响应，输出统计
- **Host-1 主程序集成**：`--mode server` 支持加载和调用 Echo Server Wasm
- **Host-2 主程序集成**：`--mode client` 支持加载和调用 Echo Client Wasm

---

## Phase 1 实现路线图

### Step 1: 创建 plugins/echo 目录结构

```bash
cd /home/cc/Desktop/code/GitHub/Ntx

# 创建主目录
mkdir -p plugins/echo/{echo-server,echo-client}

# 创建子目录
mkdir -p plugins/echo/echo-server/{src,wit}
mkdir -p plugins/echo/echo-client/{src,wit}
```

### Step 2: 实现 Echo Server Wasm

#### 2.1 创建 Cargo.toml

**文件：`plugins/echo/echo-server/Cargo.toml`**

```toml
[package]
name = "echo-server"
version = "0.1.0"
edition = "2021"

[dependencies]
wit-bindgen = { version = "0.14", features = ["reexport-macros"] }

[lib]
crate-type = ["cdylib"]

[profile.release]
opt-level = "z"     # Optimize for size
lto = true
strip = true
```

#### 2.2 创建 WIT 定义

**文件：`plugins/echo/echo-server/wit/world.wit`**

```wit
package echo:server;

interface server {
  record packet-meta {
    src-ip: list<u8>,
    dst-ip: list<u8>,
    src-port: u16,
    dst-port: u16,
    ether-type: u16,
    timestamp: u64,
  }
  
  record packet-response {
    payload: list<u8>,
    forward: bool,
  }
  
  on-packet: func(meta: packet-meta, payload: list<u8>) 
    -> result<packet-response, string>;
}

world echo-server {
  export server;
}
```

#### 2.3 实现 src/lib.rs

**文件：`plugins/echo/echo-server/src/lib.rs`**

```rust
wit_bindgen::generate!();

export!(Component);

struct Component;

impl Guest for Component {
    fn on_packet(
        meta: ServerPacketMeta,
        payload: Vec<u8>,
    ) -> Result<PacketResponse, String> {
        // Simple echo: return the same payload
        Ok(PacketResponse {
            payload,
            forward: true,
        })
    }
}
```

#### 2.4 编译

```bash
cd plugins/echo/echo-server
cargo build --target wasm32-wasip2 --release
```

### Step 3: 实现 Echo Client Wasm

#### 3.1 创建 Cargo.toml

**文件：`plugins/echo/echo-client/Cargo.toml`**

```toml
[package]
name = "echo-client"
version = "0.1.0"
edition = "2021"

[dependencies]
wit-bindgen = { version = "0.14", features = ["reexport-macros"] }

[lib]
crate-type = ["cdylib"]

[profile.release]
opt-level = "z"
lto = true
strip = true
```

#### 3.2 创建 WIT 定义

**文件：`plugins/echo/echo-client/wit/world.wit`**

```wit
package echo:client;

interface client {
  record generate-config {
    count: u32,
    pps: u32,
    dst-ip: list<u8>,
    dst-port: u16,
  }
  
  record generate-result {
    total-sent: u32,
    total-received: u32,
    matched: u32,
    timeouts: u32,
    errors: u32,
    rtt-min-us: u64,
    rtt-max-us: u64,
    rtt-avg-us: u64,
  }
  
  generate: func(config: generate-config) -> result<generate-result, string>;
}

interface host-callbacks {
  on-send-packet: func(payload: list<u8>) -> result<(), string>;
  on-recv-packet: func(timeout-ms: u32) -> result<option<list<u8>>, string>;
}

world echo-client {
  export client;
  import host-callbacks;
}
```

#### 3.3 实现 src/lib.rs

**文件：`plugins/echo/echo-client/src/lib.rs`**

```rust
wit_bindgen::generate!();

use std::time::Instant;

export!(Component);

struct Component;

impl Guest for Component {
    fn generate(config: GenerateConfig) -> Result<GenerateResult, String> {
        let mut result = GenerateResult {
            total_sent: 0,
            total_received: 0,
            matched: 0,
            timeouts: 0,
            errors: 0,
            rtt_min_us: u64::MAX,
            rtt_max_us: 0,
            rtt_avg_us: 0,
        };
        
        let mut rtt_sum: u64 = 0;
        let mut response_count = 0u32;
        
        let interval_us = if config.pps > 0 {
            1_000_000 / config.pps as u64
        } else {
            0
        };
        
        for seq in 0..config.count {
            let request = create_request_packet(seq);
            
            if let Err(_e) = on_send_packet(&request) {
                result.errors += 1;
                continue;
            }
            result.total_sent += 1;
            
            let send_time = Instant::now();
            
            match on_recv_packet(5000) {
                Ok(Some(response)) => {
                    let elapsed_us = send_time.elapsed().as_micros() as u64;
                    result.total_received += 1;
                    
                    if verify_response_token(&response, seq) {
                        result.matched += 1;
                        
                        result.rtt_min_us = result.rtt_min_us.min(elapsed_us);
                        result.rtt_max_us = result.rtt_max_us.max(elapsed_us);
                        rtt_sum += elapsed_us;
                        response_count += 1;
                    } else {
                        result.errors += 1;
                    }
                }
                Ok(None) => {
                    result.timeouts += 1;
                }
                Err(_e) => {
                    result.errors += 1;
                }
            }
            
            if interval_us > 0 && seq < config.count - 1 {
                let elapsed = send_time.elapsed().as_micros() as u64;
                if elapsed < interval_us {
                    let sleep_us = interval_us - elapsed;
                    std::thread::sleep(std::time::Duration::from_micros(sleep_us));
                }
            }
        }
        
        if response_count > 0 {
            result.rtt_avg_us = rtt_sum / response_count as u64;
        }
        
        Ok(result)
    }
}

fn create_request_packet(seq: u32) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&seq.to_be_bytes());
    packet.extend_from_slice(b"Echo test data");
    packet
}

fn verify_response_token(response: &[u8], expected_seq: u32) -> bool {
    if response.len() < 4 {
        return false;
    }
    let seq = u32::from_be_bytes([response[0], response[1], response[2], response[3]]);
    seq == expected_seq
}
```

#### 3.4 编译

```bash
cd plugins/echo/echo-client
cargo build --target wasm32-wasip2 --release
```

### Step 4: 创建 build.sh

**文件：`plugins/echo/build.sh`**

```bash
#!/bin/bash
set -e

cd "$(dirname "$0")"

echo "[*] Building echo-server..."
cd echo-server && cargo build --target wasm32-wasip2 "$@" && cd ..

echo "[*] Building echo-client..."
cd echo-client && cargo build --target wasm32-wasip2 "$@" && cd ..

echo "[+] Build complete"
```

```bash
chmod +x plugins/echo/build.sh
```

---

## Host 程序修改

### Step 5: 修改 src/main.rs 支持 --mode server 和 --mode client

详见 SCENARIO_ECHO_DESIGN.md 第 5 节 Host 集成

---

**实现指南版本：2.0 | Phase 1 | 2024-12-14**```rust
#[derive(Debug, Clone)]
pub struct Args {
    // 既有参数
    pub mode: Mode,
    pub iface: String,
    pub backend: Backend,
    pub port: u16,
    pub component: Option<String>,  // Guest 组件路径
    
    // ★ 新增参数
    pub server_mode: bool,           // 是否以 Server 模式运行
}

fn parse_args() -> Args {
    // 解析 CLI 参数
    // --component <path>     Guest 组件路径
    // --server-mode          启用 Server 模式
    
    todo!()
}
```

### Step 1.3：主循环集成 Guest 调用

在 `main()` 函数或 `run_net_mode()` 中修改主循环：

```rust
async fn run_net_mode(args: Args) -> anyhow::Result<()> {
    let nic = create_nic(&args)?;
    
    // ★ 加载 Guest 组件
    let (instance, linker) = if let Some(comp_path) = &args.component {
        load_guest_component(comp_path)?
    } else {
        eprintln!("[!] No component specified, running without guest");
        (None, None)
    };
    
    let mut stats = Stats::default();
    let mut last_report = Instant::now();
    
    loop {
        match nic.recv_nonblocking() {
            Some(buf) => {
                stats.rx_total += 1;
                
                match PacketContext::decode(&buf) {
                    Ok(ctx) if ctx.udp_meta.is_some() => {
                        stats.rx_udp += 1;
                        
                        // ★ 调用 Guest
                        if let (Some(inst), Some(link)) = (&instance, &linker) {
                            match call_guest_on_udp(link, inst, &ctx)? {
                                Some(response) => {
                                    let reply_buf = build_udp_reply(&ctx, &response)?;
                                    nic.send(&reply_buf)?;
                                    stats.tx_total += 1;
                                    stats.tx_replies += 1;
                                }
                                None => stats.rx_dropped += 1,
                            }
                        }
                    }
                    Ok(_) => { /* 非UDP */ }
                    Err(e) => {
                        stats.rx_errors += 1;
                        if env::var("NTX_DEBUG").is_ok() {
                            eprintln!("[!] Decode error: {}", e);
                        }
                    }
                }
            }
            None => {
                if !nic.poll_readable(Duration::from_millis(100))? {
                    if last_report.elapsed() > Duration::from_secs(5) {
                        println!("[stats] rx_total={} rx_udp={} rx_err={} tx_replies={} tx_total={}",
                            stats.rx_total, stats.rx_udp, stats.rx_errors,
                            stats.tx_replies, stats.tx_total);
                        last_report = Instant::now();
                    }
                }
            }
        }
    }
}
```

### Step 1.4：实现 Guest 调用包装函数

```rust
/// 根据 PacketContext 构造 PacketMeta 并调用 Guest
fn call_guest_on_udp(
    linker: &wasmtime::Linker<()>,
    instance: &wasmtime::Instance,
    ctx: &PacketContext,
) -> anyhow::Result<Option<UdpResponse>> {
    let meta = PacketMeta {
        src_ip: ctx.src_ip.to_string(),
        dst_ip: ctx.dst_ip.to_string(),
        src_port: ctx.src_port,
        dst_port: ctx.dst_port,
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs(),
    };
    
    let payload = ctx.udp_payload.to_vec();
    
    // ★ 动态调用 Guest 的 on-udp 导出
    invoke_guest_on_udp(linker, instance, meta, payload)
}

/// 加载 Guest 组件
fn load_guest_component(path: &str) -> anyhow::Result<(Option<Instance>, Option<Linker>)> {
    let engine = wasmtime::Engine::default();
    let mut linker = wasmtime::Linker::new(&engine);
    
    // 实例化组件
    let component = wasmtime::component::Component::from_file(&engine, path)?;
    let instance = linker.instantiate(&component)?;
    
    println!("[host] loaded component: {}", path);
    
    Ok((Some(instance), Some(linker)))
}
```

---

## 第 2 阶段：Guest 组件 on-udp 导出（plugins/scheduler/scheduler/src/lib.rs）

### Step 2.1：定义 WIT 接口

编辑 `plugins/scheduler/scheduler/wit/packet.wit`（如果不存在则创建）：

```wit
package scheduler:net;

interface packet {
  /// 数据包元信息
  record packet-meta {
    src-ip: string,
    dst-ip: string,
    src-port: u16,
    dst-port: u16,
    timestamp: u64,
  }
  
  /// UDP 响应
  record udp-response {
    payload: list<u8>,
    status: u16,  // 0=OK, 1=Error
  }
  
  /// 处理接收到的 UDP 包
  /// 返回 result 表示成功/失败
  /// 返回 option 表示是否生成响应（None 表示丢弃）
  on-udp: func(
    meta: packet-meta,
    payload: list<u8>
  ) -> result<option<udp-response>, string>;
}

world packet-handler {
  export packet;
}
```

### Step 2.2：在 Guest 中实现 on-udp

编辑 `plugins/scheduler/scheduler/src/lib.rs`：

```rust
use wit_bindgen::generate!();

generate!();

pub struct Component;

#[export]
impl Guest for Component {
    fn on_udp(
        meta: PacketMeta,
        payload: Vec<u8>,
    ) -> Result<Option<UdpResponse>, String> {
        // 日志
        eprintln!(
            "[guest] on-udp from {}:{} to {}:{}",
            meta.src_ip, meta.src_port,
            meta.dst_ip, meta.dst_port
        );
        
        // Step 1: 解析任务（这里用最简单的 Echo）
        let task = parse_task_from_payload(&payload)
            .map_err(|e| format!("Parse error: {}", e))?;
        
        // Step 2: 通过 EventBus 发布事件（如果已有 EventBus 组件）
        if let Err(e) = emit_packet_received_event(&meta, &payload) {
            eprintln!("[guest] EventBus emit error: {}", e);
            // 不中断处理，继续
        }
        
        // Step 3: 执行任务
        match execute_task(&task) {
            Ok(result_payload) => {
                let response = UdpResponse {
                    payload: result_payload,
                    status: 0,  // OK
                };
                eprintln!("[guest] on-udp success, reply {} bytes", response.payload.len());
                Ok(Some(response))
            }
            Err(e) => {
                eprintln!("[guest] Task execution error: {}", e);
                Err(e)
            }
        }
    }
}

/// 从 payload 解析任务
fn parse_task_from_payload(payload: &[u8]) -> Result<Task, String> {
    // 最简单情况：整个 payload 就是数据
    // 可以扩展为支持更复杂的协议头
    
    Ok(Task::Echo(payload.to_vec()))
}

/// 执行任务（使用 Scheduler + Actions）
fn execute_task(task: &Task) -> Result<Vec<u8>, String> {
    match task {
        Task::Echo(data) => {
            // 直接回显
            Ok(data.clone())
        }
        Task::Transform(op, data) => {
            // 使用 Scheduler 处理转换
            // TODO: 集成 Scheduler 的状态机
            match op.as_str() {
                "upper" => {
                    let s = String::from_utf8_lossy(data);
                    Ok(s.to_uppercase().into_bytes())
                }
                "lower" => {
                    let s = String::from_utf8_lossy(data);
                    Ok(s.to_lowercase().into_bytes())
                }
                _ => Err(format!("Unknown transform: {}", op)),
            }
        }
    }
}

/// 发布事件到 EventBus（如果已集成）
fn emit_packet_received_event(
    meta: &PacketMeta,
    payload: &[u8],
) -> Result<(), String> {
    // TODO: 调用 EventBus 组件的导出接口
    // 这里暂时是 no-op
    Ok(())
}

#[derive(Debug)]
enum Task {
    Echo(Vec<u8>),
    Transform(String, Vec<u8>),
}
```

### Step 2.3：更新 Cargo.toml 和构建

编辑 `plugins/scheduler/scheduler/Cargo.toml`：

```toml
[package]
name = "scheduler"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[package.metadata.component]
package = "scheduler:net"

[package.metadata.component.target]
path = "wit"
world = "packet-handler"

[dependencies]
wit-bindgen = { version = "0.49.0", features = ["realloc"] }
anyhow = "1.0"
thiserror = "2.0"
serde_json = "1.0"
serde = { version = "1.0", features = ["derive"] }
```

构建命令：

```bash
cd plugins/scheduler/scheduler
cargo build --target wasm32-wasip2
```

输出：`target/wasm32-wasip2/debug/scheduler.wasm`

---

## 第 3 阶段：WAC 组件组装 & 组件策略选择

### Step 3.0：理解组件组装的两种策略

**问题**：我们需要的是单一 wasm 文件还是多个？

**答案**：取决于你的场景和需求。这里提供两种官方策略对比：

#### 🏗️ 策略 A：多组件导出（modular - 模块化）

**架构**：
```
echo_scenario.wac
  ├─ core-libs.wasm
  ├─ eventbus.wasm
  ├─ actions-executor.wasm
  └─ scheduler.wasm
       ↓
   echo_composed.wasm (包含 4 个导出)
```

**.wac 文件**：
```plaintext
package scheduler:echo;

let core = new component "file://../core-libs/target/wasm32-wasip2/debug/scheduler_core_libs.wasm";
let eventbus = new component "file://../eventbus/target/wasm32-wasip2/debug/scheduler_eventbus.wasm";
let actions = new component "file://../actions-executor/target/wasm32-wasip2/debug/scheduler_actions_executor.wasm";
let scheduler = new component "file://../scheduler/target/wasm32-wasip2/debug/scheduler.wasm";

export core;
export eventbus;
export actions;
export scheduler;
```

**生成命令**：
```bash
wac plug echo_scenario.wac -o echo_composed.wasm
wasmtime inspect echo_composed.wasm
# 输出包含多个导出
```

**优点** ✅：
- 各组件独立编译、测试、升级
- 支持运行时组件替换
- 模块职责清晰
- 易于扩展功能

**缺点** ❌：
- 组件间调用开销稍大
- Host 需要处理多个导出的查找
- .wasm 文件稍大（多个组件头部）
- 调试复杂度增加

**适用场景**：
- 长期维护的生产系统
- 需要频繁升级某个模块
- 多个不同场景复用

---

#### 🎯 策略 B：单一组合组件（unified - 统一）

**架构**：
```
Scheduler.wasm (内部链接所有库)
  ├─ core-libs
  ├─ eventbus
  └─ actions-executor
       ↓ (直接编译或通过 wasm-tools compose)
   echo_composed.wasm (单一导出)
```

**生成方式 1：直接编译单一 wasm**

```bash
# Scheduler 在 Cargo.toml 中依赖其他 crate（非 wasm 版本）
cd plugins/scheduler/scheduler
cargo build --target wasm32-wasip2
# 输出：scheduler.wasm（包含所有功能）
```

**生成方式 2：使用 wasm-tools 合并**

```bash
# 先编译各组件为 wasm
cd plugins/scheduler && \
  cargo build --target wasm32-wasip2 -p scheduler-core && \
  cargo build --target wasm32-wasip2 -p scheduler-eventbus && \
  cargo build --target wasm32-wasip2 -p scheduler-actions-executor && \
  cargo build --target wasm32-wasip2 -p scheduler

# 合并为单一文件
wasm-tools compose \
  core-libs.wasm \
  eventbus.wasm \
  actions-executor.wasm \
  scheduler.wasm \
  -o echo_composed.wasm
```

**优点** ✅：
- 部署最简洁（只有 1 个 .wasm）
- Host 代码最简单
- 性能最优（无组件间 RPC 开销）
- 文件最小（无重复头部）
- 最适合 MVP/演示

**缺点** ❌：
- 组件紧耦合，不易单独升级
- 调试困难（所有代码都在一个文件中）
- 不支持运行时替换

**适用场景**：
- MVP 演示和验证
- 内部工具
- 性能敏感的系统
- 功能相对稳定的场景

---

#### 📊 对比表

| 维度 | 策略 A（多组件） | 策略 B（单一） |
|------|-----------------|-----------|
| **文件数量** | 1 个 .wasm（含 4 导出） | 1 个 .wasm（单一导出） |
| **总大小** | 稍大（多头部） | 最小 |
| **编译时间** | 稍快（可并行） | 稍慢（需合并） |
| **运行性能** | 92% | 100% (基准) |
| **模块独立性** | 高 | 低 |
| **易维护性** | 高 | 中 |
| **易部署性** | 中 | 高 |
| **易扩展性** | 高 | 低 |
| **推荐场景** | 生产系统 | MVP/演示 |

---

#### 🎯 **当前推荐：策略 B（单一组件）**

**理由**：
1. Echo 场景是 MVP，首要目标是验证端到端链路
2. 代码实现和调试更直接
3. Host 端最简单
4. 最适合展示功能

**具体步骤**：

### Step 3.1：创建 WAC 配置（策略 B 风格）

编辑 `plugins/scheduler/wac/echo_scenario.wac`：

```plaintext
package scheduler:echo;

// 导入主 Scheduler 组件（已链接所有依赖）
let scheduler = new component "file://../scheduler/target/wasm32-wasip2/debug/scheduler.wasm";

// 导出单一接口
export scheduler;
```

### Step 3.2：编译并生成单一 wasm

```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler

# Step 1: 编译 Scheduler（会自动链接其他库）
cd scheduler
cargo build --target wasm32-wasip2

# Step 2: 使用 wac 包裹成标准组件格式（可选）
cd ../wac
wac plug echo_scenario.wac -o echo_composed.wasm

# 或者直接使用 scheduler.wasm（已是有效组件）
# cp ../scheduler/target/wasm32-wasip2/debug/scheduler.wasm echo_composed.wasm

# Step 3: 验证最终 wasm
wasmtime inspect echo_composed.wasm | grep -E "export|on-udp|packet"
```

**预期输出**：
```
exports:
  "scheduler:net/packet": ...
    on-udp: func(...)
```

### Step 3.3：Host 端的简洁调用

由于只有单一导出，Host 代码可以非常简洁：

```rust
fn load_guest_component(path: &str) -> anyhow::Result<Instance> {
    let engine = wasmtime::Engine::default();
    let mut linker = wasmtime::Linker::new(&engine);
    
    let component = wasmtime::component::Component::from_file(&engine, path)?;
    let instance = linker.instantiate(&component)?;
    
    println!("[host] loaded component: {}", path);
    Ok(instance)
}

// 调用也很直接：
match instance.exports().get("packet")?.call_on_udp(meta, payload)? {
    Ok(Some(response)) => { /* 处理响应 */ }
    _ => { /* 处理错误或无响应 */ }
}
```

---

#### 🚀 后续升级路径

如果后续需要模块化（从策略 B 升级到策略 A）：

```bash
# 只需修改 wac 文件和 Host 调用方式，无需改变组件代码
# 1. 在 wac 中同时导出多个组件
# 2. 在 Host 中添加多个导出的查找逻辑
# 3. 完成！

# 代码改动最小化
```

---

## 第 4 阶段：端到端集成

### Step 4.1：编译

```bash
cd /home/cc/Desktop/code/GitHub/Ntx

# 编译主程序
cargo build

# 编译 traffic-send
cargo build --examples

# 编译 Guest 组件
cd plugins/scheduler/scheduler
cargo build --target wasm32-wasip2
cd ../wac
wac plug echo_scenario.wac -o echo_composed.wasm
```

### Step 4.2：运行完整场景

```bash
# 使用自动化脚本
sudo ./scripts/ntx-e2e-echo.sh --tcpdump

# 或手动分步

# 第 1 步：启动网络拓扑
sudo ./scripts/ntx-veth-up.sh

# 第 2 步：启动 Host-1（另开终端）
timeout 30 sudo ./scripts/ntxns1.sh \
  ./target/debug/Ntx \
  --mode net \
  --iface ntx0 \
  --backend afpacket-dgram \
  --port 10001 \
  --component ./plugins/scheduler/wac/echo_composed.wasm

# 第 3 步：启动 Host-2（第三个终端）
sudo ./scripts/ntxns2.sh \
  ./target/debug/examples/traffic-send \
  --iface ntx1 \
  --backend afpacket-dgram \
  --dst-ips 10.0.0.1 \
  --src-ip 10.0.0.2 \
  --dst-port 10001 \
  --src-port 40000 \
  --rr \
  --count 20
```

### Step 4.3：验证输出

**Host-1 应输出：**
```
[host] loaded component: ./plugins/scheduler/wac/echo_composed.wasm
[host] listening on 10.0.0.1:10001
[guest] on-udp from 10.0.0.2:40000 to 10.0.0.1:10001
[guest] on-udp success, reply XX bytes
[stats] rx_total=20 rx_udp=20 rx_err=0 tx_replies=20 tx_total=20
```

**Host-2 应输出：**
```
[tx] seq=1 dst_ip=10.0.0.1:10001 token=abc123
[rx] seq=1 from=10.0.0.1:10001 token=abc123 ✓ matched
[final] sent=20 matched=20 timeouts=0
exit code: 0
```

---

## 常见问题

### Q1: Guest on-udp 导出不被识别

**症状：** Host 启动时找不到 `on-udp` 导出。

**排查：**
1. 确认 WIT 文件已定义 `packet` 接口和 `on-udp` 函数。
2. 确认 `Cargo.toml` 中 `package.metadata.component` 指向正确的 WIT 目录。
3. 重新编译：`cargo clean && cargo build --target wasm32-wasip2`。
4. 验证 wasm 导出：`wasmtime inspect scheduler.wasm | grep on-udp`

### Q2: WAC 组合失败

**症状：** `wac plug` 报错。

**排查：**
1. 确认所有子组件已编译为 wasm32-wasip2。
2. 确认 .wac 文件中的路径正确。
3. 运行 `wac plug -v` 查看详细错误。

### Q3: Guest 调用时 Wasmtime 错误

**症状：** `Val` 编码或解码错误。

**排查：**
1. 参见 `src/guest_packet_val.rs`，确认 `Val` 编码逻辑已更新。
2. 检查 Wasmtime 版本：`wasmtime --version`。
3. 使用 `NTX_DEBUG=1` 查看详细错误。

### Q4: 没有看到 Guest 日志

**症状：** 主程序运行但不输出 Guest 中的 eprintln! 内容。

**排查：**
1. 确认 Host 正确捕获和打印 Guest 的 stderr。
2. 考虑使用 `env_logger` 或其他日志库而非 eprintln!。
3. 用 `wasmtime run` 单独测试组件。

---

## 下一步扩展

### 集成 EventBus

一旦 EventBus 组件完成，修改 `emit_packet_received_event()`：

```rust
fn emit_packet_received_event(
    meta: &PacketMeta,
    payload: &[u8],
) -> Result<(), String> {
    // 调用 EventBus 导出的 emit 函数
    eventbus::emit(Event::PacketReceived {
        src_ip: meta.src_ip.clone(),
        dst_ip: meta.dst_ip.clone(),
        payload: payload.to_vec(),
    })?;
    Ok(())
}
```

### 集成 Scheduler

使用 Scheduler 的状态机处理更复杂的任务流程：

```rust
fn execute_task(task: &Task) -> Result<Vec<u8>, String> {
    // 通过 Scheduler 的 WIT 接口创建和执行任务
    scheduler::run_task(task)?;
    // 获取结果
    Ok(scheduler::get_result()?)
}
```

---

**文档版本：1.0 | 更新时间：2024-12-14**
