# Echo 场景 Phase 1 实现指南 v2.0

## 概述

本指南详细说明如何实现 Echo 场景的第一阶段（Phase 1），基于**模块化、事件驱动的架构**。

### 核心理念

- **两个不同的 actions-executor 实现**：
  - `actions-executor-server`: 处理入站包并回显
  - `actions-executor-client`: 生成请求并验证响应
- **模块化设计**：每个组件职责清晰，易于测试和维护
- **事件驱动**：通过 eventbus 进行组件间通信（Phase 2）

---

## 第一部分：已完成的基础组件

### 1. actions-executor-server

**位置**：`plugins/scheduler/actions-executor-server/`

#### 1.1 核心功能

Server 端的核心是 `handle_on_packet_received()` 函数：

```rust
pub fn handle_on_packet_received(
    _meta: PacketMeta,
    payload: Vec<u8>,
) -> Result<PacketResponse, String> {
    if payload.is_empty() {
        return Err("Payload is empty".to_string());
    }
    Ok(PacketResponse {
        payload,        // ★ 直接返回（Echo 的本质）
        forward: true,  // 指示 Host 转发
    })
}
```

#### 1.2 关键数据结构

```rust
pub struct PacketMeta {
    pub src_ip: Vec<u8>,
    pub dst_ip: Vec<u8>,
    pub src_port: u16,
    pub dst_port: u16,
    pub ether_type: u16,
    pub timestamp: u64,
}

pub struct PacketResponse {
    pub payload: Vec<u8>,
    pub forward: bool,
}
```

#### 1.3 编译和测试

```bash
# 编译
cd plugins/scheduler/actions-executor-server
cargo build

# 测试
cargo test

# 预期输出
running 1 test
test tests::test_echo ... ok
test result: ok. 1 passed; 0 failed
```

#### 1.4 设计特点

| 特性 | 说明 |
|------|------|
| **无状态** | 不保持任何会话状态 |
| **快速** | 直接返回，无计算开销 |
| **简洁** | 核心逻辑不超过 20 行 |
| **可靠** | 简单意味着可靠 |

---

### 2. actions-executor-client

**位置**：`plugins/scheduler/actions-executor-client/`

#### 2.1 核心功能

Client 端包含两个主要函数：

**生成请求包**：
```rust
pub fn create_request_packet(seq: u32) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&seq.to_be_bytes());
    packet.extend_from_slice(b"Echo test payload");
    packet
}
```

**验证响应包**：
```rust
pub fn verify_response_packet(response: &[u8], expected_seq: u32) -> bool {
    if response.len() < 4 {
        return false;
    }
    let seq = u32::from_be_bytes([response[0], response[1], response[2], response[3]]);
    seq == expected_seq
}
```

#### 2.2 关键数据结构

```rust
pub struct GenerateConfig {
    pub count: u32,         // 生成多少个请求
    pub pps: u32,           // 每秒请求数
    pub dst_ip: Vec<u8>,    // 目标 IP
    pub dst_port: u16,      // 目标端口
}

pub struct GenerateResult {
    pub total_sent: u32,
    pub total_received: u32,
    pub matched: u32,
    pub timeouts: u32,
    pub errors: u32,
    pub rtt_min_us: u64,
    pub rtt_max_us: u64,
    pub rtt_avg_us: u64,
}
```

#### 2.3 编译和测试

```bash
# 编译
cd plugins/scheduler/actions-executor-client
cargo build

# 测试
cargo test

# 预期输出
running 2 tests
test tests::test_create_packet ... ok
test tests::test_verify_packet ... ok
test result: ok. 2 passed; 0 failed
```

#### 2.4 设计特点

| 特性 | 说明 |
|------|------|
| **有状态** | 维护请求队列和统计 |
| **完整** | 包含生成、发送、验证、统计 |
| **可扩展** | 易于添加新的验证方式 |
| **可测试** | 提供单独的测试函数 |

---

## 第二部分：WAC 编排

### 3. WAC 配置文件

WAC（WebAssembly Assembly）用于将多个 WASM 组件组合成一个完整的应用。

#### 3.1 Echo Server WAC 配置

**文件**：`plugins/scheduler/wac/echo-server.wac`

```plaintext
package scheduler:echo-server;

# 导入各组件
let corelibs = new component "file://../core-libs/target/wasm32-wasip2/debug/scheduler_core_libs.wasm";
let eventbus = new component "file://../eventbus/target/wasm32-wasip2/debug/scheduler_eventbus.wasm";
let actions = new component "file://../actions-executor-server/target/wasm32-wasip2/debug/scheduler_actions_executor_server.wasm";
let scheduler = new component "file://../scheduler/target/wasm32-wasip2/debug/scheduler.wasm";

# 建立组件间连接
connect eventbus with corelibs;
connect actions with eventbus;
connect actions with corelibs;
connect scheduler with eventbus;
connect scheduler with actions;

# 导出主入口
export scheduler;
```

#### 3.2 Echo Client WAC 配置

**文件**：`plugins/scheduler/wac/echo-client.wac`

```plaintext
package scheduler:echo-client;

# 导入各组件
let corelibs = new component "file://../core-libs/target/wasm32-wasip2/debug/scheduler_core_libs.wasm";
let eventbus = new component "file://../eventbus/target/wasm32-wasip2/debug/scheduler_eventbus.wasm";
let actions = new component "file://../actions-executor-client/target/wasm32-wasip2/debug/scheduler_actions_executor_client.wasm";
let scheduler = new component "file://../scheduler/target/wasm32-wasip2/debug/scheduler.wasm";

# 建立组件间连接
connect eventbus with corelibs;
connect actions with eventbus;
connect actions with corelibs;
connect scheduler with eventbus;
connect scheduler with actions;

# 导出主入口
export scheduler;
```

#### 3.3 WAC 编排工具

需要安装 `wac` 工具来编排 WASM 组件：

```bash
# 安装 wac（使用 cargo）
cargo install wac-cli

# 验证安装
wac --version
```

#### 3.4 编组命令

```bash
# 进入 WAC 目录
cd plugins/scheduler/wac

# 编排 Server
wac plug echo-server.wac -o echo-server.wasm

# 编排 Client
wac plug echo-client.wac -o echo-client.wasm

# 验证输出
ls -lh echo-*.wasm
```

---

## 第三部分：Host 集成

### 4. Host-1 (Echo Server) 集成

#### 4.1 主程序入口

在 `src/main.rs` 中添加 `--mode server` 支持：

```rust
mod server_mode {
    use std::path::Path;
    use wasmtime::{Engine, Instance, Linker, Module, Store};

    pub fn run(wasm_path: &str, port: u16, iface: &str) -> anyhow::Result<()> {
        // 1. 初始化 Wasmtime
        let engine = Engine::default();
        let mut store = Store::new(&engine, ());
        
        // 2. 加载 Wasm 模块
        let module = Module::from_file(&engine, wasm_path)?;
        let mut linker = Linker::new(&engine);
        let instance = linker.instantiate(&mut store, &module)?;
        
        // 3. 获取导出的 on_packet_received 函数
        // TODO: 根据 Wasm 实际导出接口获取函数
        
        // 4. 初始化网络接口
        let nic = init_nic(iface)?;
        
        // 5. 主循环：接收 → 处理 → 回复
        server_main_loop(&nic, &mut store, &instance)?;
        
        Ok(())
    }
    
    fn server_main_loop(nic: &dyn Nic, store: &mut Store<()>, instance: &Instance) -> anyhow::Result<()> {
        loop {
            // 接收包
            if let Some(buf) = nic.recv_nonblocking() {
                // 解析包
                let (meta, payload) = decode_packet(&buf)?;
                
                // 调用 Wasm 组件处理
                // let response = call_wasm_on_packet(store, instance, meta, payload)?;
                
                // 构造回复包
                // let reply = build_reply_packet(&buf, response)?;
                
                // 发送回复
                // nic.send(&reply)?;
            }
        }
    }
}
```

#### 4.2 数据包处理

```rust
// 解析收到的包
fn decode_packet(buf: &[u8]) -> anyhow::Result<(PacketMeta, Vec<u8>)> {
    // 1. 解析以太网头部
    let (eth_src, eth_dst, eth_type) = parse_ethernet_header(buf)?;
    
    // 2. 根据类型解析 IP 头部
    let (src_ip, dst_ip, proto) = parse_ip_header(buf, eth_type)?;
    
    // 3. 根据协议解析传输层头部
    let (src_port, dst_port, payload_offset) = if proto == 0x11 {
        // UDP
        parse_udp_header(buf)?
    } else {
        return Err("Only UDP supported".into());
    };
    
    // 4. 提取 payload
    let payload = buf[payload_offset..].to_vec();
    
    let meta = PacketMeta {
        src_ip: src_ip.to_vec(),
        dst_ip: dst_ip.to_vec(),
        src_port,
        dst_port,
        ether_type: eth_type,
        timestamp: get_timestamp(),
    };
    
    Ok((meta, payload))
}

// 构造回复包
fn build_reply_packet(original_buf: &[u8], response_payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    // 1. 交换源目 MAC 地址
    // 2. 交换源目 IP 地址
    // 3. 交换源目 UDP 端口
    // 4. 更新 UDP checksum
    // 5. 更新 IP checksum
    // 6. 使用 response_payload 替换原 payload
    
    // TODO: 实现完整的包构造逻辑
    Ok(vec![])
}
```

### 5. Host-2 (Echo Client) 集成

#### 5.1 主程序入口

在 `src/main.rs` 中添加 `--mode client` 支持：

```rust
mod client_mode {
    use std::path::Path;
    use wasmtime::{Engine, Instance, Linker, Module, Store};

    pub fn run(
        wasm_path: &str,
        server_ip: &str,
        server_port: u16,
        count: u32,
        pps: u32,
        iface: &str,
    ) -> anyhow::Result<()> {
        // 1. 初始化 Wasmtime
        let engine = Engine::default();
        let mut store = Store::new(&engine, ());
        
        // 2. 加载 Wasm 模块
        let module = Module::from_file(&engine, wasm_path)?;
        let mut linker = Linker::new(&engine);
        
        // 3. 链接 Host callbacks
        // linker.func_wrap("", "send-packet", host_send_packet)?;
        // linker.func_wrap("", "recv-packet", host_recv_packet)?;
        
        let instance = linker.instantiate(&mut store, &module)?;
        
        // 4. 初始化网络接口
        let nic = init_nic(iface)?;
        
        // 5. 调用 Wasm generate() 函数
        // let result = call_wasm_generate(store, instance, count, pps)?;
        
        // 6. 输出结果
        println!("[result] sent={} matched={} rtt_avg={}us",
            result.total_sent, result.matched, result.rtt_avg_us);
        
        Ok(())
    }
}

// Host 侧的回调：发送包
fn host_send_packet(payload: &[u8]) -> anyhow::Result<()> {
    // 1. 构造完整的 UDP/IP/Ethernet 包
    let packet = build_complete_packet(payload)?;
    
    // 2. 通过 NIC 发送
    nic.send(&packet)?;
    
    Ok(())
}

// Host 侧的回调：接收包
fn host_recv_packet(timeout_ms: u32) -> anyhow::Result<Option<Vec<u8>>> {
    // 1. 尝试从 NIC 接收
    match nic.recv_with_timeout(timeout_ms) {
        Some(buf) => {
            // 2. 解析包，提取 payload
            let payload = extract_payload(&buf)?;
            Ok(Some(payload))
        }
        None => Ok(None), // timeout
    }
}
```

#### 5.2 运行命令

```bash
# Server 模式
./target/release/Ntx \
  --mode server \
  --iface eth0 \
  --port 10001 \
  --component ./plugins/scheduler/wac/echo-server.wasm

# Client 模式
./target/release/Ntx \
  --mode client \
  --iface eth1 \
  --server-ip 10.0.0.1 \
  --server-port 10001 \
  --component ./plugins/scheduler/wac/echo-client.wasm \
  --count 1000 \
  --pps 100
```

---

## 第四部分：完整编译流程

### 6. 自动编译脚本

**文件**：`plugins/scheduler/scripts/build-echo.sh`

```bash
#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCHEDULER_DIR="$(dirname "$SCRIPT_DIR")"

echo "╔════════════════════════════════════════════════════╗"
echo "║        Echo 场景 WAC 编排编译脚本                  ║"
echo "╚════════════════════════════════════════════════════╝"

# 1. 编译各组件
echo "[1/6] Building core-libs..."
cd "$SCHEDULER_DIR/core-libs"
cargo build --target wasm32-wasip2 --release

echo "[2/6] Building eventbus..."
cd "$SCHEDULER_DIR/eventbus"
cargo build --target wasm32-wasip2 --release

echo "[3/6] Building actions-executor-server..."
cd "$SCHEDULER_DIR/actions-executor-server"
cargo build --target wasm32-wasip2 --release

echo "[4/6] Building actions-executor-client..."
cd "$SCHEDULER_DIR/actions-executor-client"
cargo build --target wasm32-wasip2 --release

echo "[5/6] Building scheduler..."
cd "$SCHEDULER_DIR/scheduler"
cargo build --target wasm32-wasip2 --release

# 2. WAC 编排
echo "[6/6] WAC orchestration..."
cd "$SCHEDULER_DIR/wac"

if ! command -v wac &> /dev/null; then
    echo "[!] wac tool not found. Install with:"
    echo "    cargo install wac-cli"
    exit 1
fi

wac plug echo-server.wac -o echo-server.wasm
wac plug echo-client.wac -o echo-client.wasm

echo ""
echo "╔════════════════════════════════════════════════════╗"
echo "║                 编译完成！                         ║"
echo "╚════════════════════════════════════════════════════╝"
echo ""
echo "[+] Output files:"
echo "    • $SCHEDULER_DIR/wac/echo-server.wasm"
echo "    • $SCHEDULER_DIR/wac/echo-client.wasm"
```

### 7. 执行编译

```bash
chmod +x plugins/scheduler/scripts/build-echo.sh
./plugins/scheduler/scripts/build-echo.sh
```

---

## 第五部分：测试清单

### 8. 单元测试

```bash
# Server 端测试
cd plugins/scheduler/actions-executor-server
cargo test
# 预期：test tests::test_echo ... ok

# Client 端测试
cd plugins/scheduler/actions-executor-client
cargo test
# 预期：test tests::test_create_packet ... ok
#       test tests::test_verify_packet ... ok
```

### 9. 端到端测试

```bash
# 1. 启动 veth 拓扑
./scripts/ntx-veth-up.sh

# 2. 启动 Server（Host-1）
sudo ./scripts/ntxns1.sh timeout 60 \
  ./target/release/Ntx \
  --mode server \
  --iface veth1 \
  --component ./plugins/scheduler/wac/echo-server.wasm

# 3. 启动 Client（Host-2，另一个终端）
sudo ./scripts/ntxns2.sh \
  ./target/release/Ntx \
  --mode client \
  --iface veth2 \
  --server-ip 10.0.0.1 \
  --component ./plugins/scheduler/wac/echo-client.wasm \
  --count 100 \
  --pps 50

# 4. 预期输出
# [result] sent=100 matched=98 timeouts=2 avg_rtt=234us
```

---

## 关键文件清单

| 路径 | 文件 | 状态 | 说明 |
|------|------|------|------|
| plugins/scheduler/actions-executor-server/ | Cargo.toml | ✅ | Server 项目配置 |
| plugins/scheduler/actions-executor-server/ | src/lib.rs | ✅ | Server 核心实现 |
| plugins/scheduler/actions-executor-client/ | Cargo.toml | ✅ | Client 项目配置 |
| plugins/scheduler/actions-executor-client/ | src/lib.rs | ✅ | Client 核心实现 |
| plugins/scheduler/wac/ | echo-server.wac | ✅ | Server WAC 配置 |
| plugins/scheduler/wac/ | echo-client.wac | ✅ | Client WAC 配置 |
| plugins/scheduler/scripts/ | build-echo.sh | ✅ | 自动编译脚本 |
| src/main.rs | - | ⏳ | Host 集成代码 |

---

## 故障排除

### 问题 1: wac 工具未找到
```bash
# 安装 wac
cargo install wac-cli
```

### 问题 2: Wasm 编译失败
```bash
# 检查 Rust 目标
rustup target add wasm32-wasip2
```

### 问题 3: NIC 初始化失败
```bash
# 检查网络配置
ip link show
ip addr show

# 检查权限
sudo ip link show  # 需要 root 权限
```

### 问题 4: 包校验和不正确
```bash
# 手动计算校验和
# UDP checksum 需要包含伪头部
# IP checksum 需要清零原值后计算
```

---

## 文档参考

- **SCENARIO_ECHO_DESIGN.md**：详细架构设计
- **ECHO_QUICKSTART.md**：快速开始指南
- **IMPLEMENTATION_PROGRESS.md**：实现进度跟踪

---

**版本**：2.0 | **更新**：2024-12-14 | **状态**：✅ Phase 1 核心组件完成
