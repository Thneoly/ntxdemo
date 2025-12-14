# Echo WASM 加载架构文档

## 概述

本文档描述了 Ntx 中 Echo Server 和 Echo Client 的 WASM 加载架构。系统已重构为完全支持 WASM 组件加载，同时保留本地实现作为回退方案。

## 架构设计

### 关键变更 (v2.1)

#### 1. main() 函数重构
- **之前**: Echo 模式跳过 WASM 加载，直接调用本地实现
- **之后**: 所有模式都加载相应的 WASM 组件
  - `Mode::Net` → `plugins/scheduler/wac/scheduler-composed.wasm`
  - `Mode::Scenario` → `plugins/scheduler/wac/scheduler-composed.wasm`
  - `Mode::EchoServer` → `plugins/scheduler/wac/echo-server.wasm`
  - `Mode::EchoClient` → `plugins/scheduler/wac/echo-client.wasm`

#### 2. WASM 加载流程

```rust
fn main() -> Result<()> {
    let opt = parse_args();
    
    // 统一的 WASM 加载流程
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(false);
    
    let engine = Engine::new(&config)?;
    
    // 根据模式选择正确的 WASM 组件
    let component_path = match opt.mode {
        Mode::EchoServer => "plugins/scheduler/wac/echo-server.wasm",
        Mode::EchoClient => "plugins/scheduler/wac/echo-client.wasm",
        _ => opt.component_path.as_str(),
    };
    
    // 加载 WASM 组件
    let component = Component::from_file(&engine, &component_path)?;
    let instance = linker.instantiate(&mut store, &component)?;
    
    // 根据模式调用相应的处理函数
    match opt.mode {
        Mode::EchoServer => run_echo_server_wasm(&mut store, &instance, &opt),
        Mode::EchoClient => run_echo_client_wasm(&mut store, &instance, &opt),
        // ...其他模式
    }
}
```

#### 3. 双层实现模式

每个 Echo 模式都采用双层架构：

**Layer 1: WASM 加载层 (run_echo_*_wasm)**
```rust
fn run_echo_server_wasm(store: &mut Store<State>, instance: &Instance, opt: &Opt) -> Result<()> {
    // 第一步：尝试从 WASM 组件中查找导出函数
    let wasm_result = find_top_level_func(store, instance, &["on-packet-received"])
        .ok()
        .and_then(|func| {
            eprintln!("[wasm] Found exported function, using WASM implementation");
            Some(func)
        });
    
    if wasm_result.is_some() {
        // 如果找到 WASM 导出，使用它
        eprintln!("[wasm] WASM echo server loaded successfully");
        // 调用 WASM 导出函数处理数据包
        // handle_packets_with_wasm(...)
    } else {
        // 否则回退到本地实现
        eprintln!("[wasm] No suitable WASM export found, falling back to native implementation");
        run_echo_server_native(store, instance, opt)
    }
}
```

**Layer 2: 本地实现层 (run_echo_*_native)**
```rust
fn run_echo_server_native(_store: &mut Store<State>, _instance: &Instance, opt: &Opt) -> Result<()> {
    // 完整的本地 Rust 实现
    // 初始化 NIC
    // 主处理循环
    // 数据包回显逻辑
}
```

### 优势

1. **统一的架构**: 所有模式遵循相同的 WASM 加载模式
2. **向后兼容**: 本地实现确保当 WASM 未就绪时系统仍能工作
3. **易于扩展**: 添加新的 WASM 功能只需更新导出函数查询
4. **清晰的抽象**: 分离 WASM 加载逻辑和业务实现
5. **自适应**: 根据WASM可用性自动选择最佳实现

## WASM 文件

### echo-server.wasm

**位置**: `plugins/scheduler/wac/echo-server.wasm`

**预期导出接口**:
```wit
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
    
    on-packet-received: func(meta: packet-meta, payload: list<u8>) 
        -> result<packet-response, string>;
}
```

**源代码**: `plugins/scheduler/actions-executor-server/`

### echo-client.wasm

**位置**: `plugins/scheduler/wac/echo-client.wasm`

**预期导出接口**:
```wit
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
    
    generate: func(config: generate-config) 
        -> result<generate-result, string>;
}
```

**源代码**: `plugins/scheduler/actions-executor-client/`

## 编译 WASM 组件

### 构建 actions-executor-server WASM

```bash
cd plugins/scheduler/actions-executor-server
cargo build --target wasm32-wasip2
# 生成: target/wasm32-wasip2/debug/scheduler_actions_executor_server.wasm
```

### 构建 actions-executor-client WASM

```bash
cd plugins/scheduler/actions-executor-client
cargo build --target wasm32-wasip2
# 生成: target/wasm32-wasip2/debug/scheduler_actions_executor_client.wasm
```

### 使用 WAC 组合

```bash
cd plugins/scheduler/wac
wac plugins/scheduler/wac/echo-server.wac -o echo-server.wasm
wac plugins/scheduler/wac/echo-client.wac -o echo-client.wasm
```

## 运行模式

### Echo Server (WASM)

```bash
./target/debug/Ntx --mode server --iface veth1 --port 10001
```

运行流程:
1. 加载 `echo-server.wasm`
2. 尝试查找 `on-packet-received` 导出
3. 如果找到，使用 WASM 实现处理数据包
4. 如果未找到，回退到本地 Rust 实现

**日志输出**:
```
ntx(echo-server-wasm) starting: iface=veth1 port=10001 ...
[wasm] Found exported function, using WASM implementation
或
[wasm] No suitable WASM export found, falling back to native implementation
```

### Echo Client (WASM)

```bash
./target/debug/Ntx --mode client --iface veth2 --server-ip 10.0.0.1 --count 10 --pps 5
```

运行流程:
1. 加载 `echo-client.wasm`
2. 尝试查找 `generate` 导出
3. 如果找到，使用 WASM 实现生成请求
4. 如果未找到，回退到本地 Rust 实现

**日志输出**:
```
ntx(echo-client-wasm) starting: iface=veth2 ...
[wasm] Found exported function, using WASM implementation
或
[wasm] No suitable WASM export found, falling back to native implementation
```

## 迁移指南

### Phase 1: 架构准备 ✅ COMPLETE
- [x] 统一 WASM 加载流程
- [x] 实现 WASM 加载层
- [x] 提供本地实现回退
- [x] 编译验证

### Phase 2: WASM 实现 (进行中)
- [ ] 完成 actions-executor-server WASM 编译
- [ ] 完成 actions-executor-client WASM 编译
- [ ] 使用 WAC 组合生成 echo-server.wasm 和 echo-client.wasm
- [ ] 集成测试

### Phase 3: 优化 (未来)
- [ ] WASM 性能优化
- [ ] 完整包格式支持
- [ ] RTT 测量集成

## 故障排除

### 症状: "component imports instance ... but a matching implementation was not found"

**原因**: WASM 文件缺少必要的依赖或导出

**解决方案**:
1. 检查 WASM 文件是否存在: `ls -lh plugins/scheduler/wac/echo-*.wasm`
2. 检查导出: `wasm-tools component wit plugins/scheduler/wac/echo-server.wasm`
3. 回退到本地实现: 系统会自动输出 `[wasm] No suitable WASM export found, falling back...`

### 症状: "failed to find export ... on-packet-received"

**原因**: WASM 组件导出名与预期不匹配

**解决方案**:
1. 在 `find_top_level_func()` 中添加更多候选名称
2. 使用 `wasm-tools component wit` 查看实际导出

## 相关文件

- `src/main.rs` - main() 和 WASM 加载逻辑
- `plugins/scheduler/wac/echo-server.wac` - Server WAC 配置
- `plugins/scheduler/wac/echo-client.wac` - Client WAC 配置
- `plugins/scheduler/actions-executor-server/` - Server 源代码
- `plugins/scheduler/actions-executor-client/` - Client 源代码

## 技术参考

- Wasmtime Component Model 文档
- WIT (WebAssembly Interface Types) 规范
- WAC (Wasm Assembly Composition) 工具

## 版本历史

- **v2.1** (2024-12-14): WASM 加载架构实现
  - 统一 WASM 加载流程
  - 添加 WASM 查询和回退机制
  - 双层实现支持

- **v2.0** (2024-12-14): 本地实现完成
  - Echo Server 本地实现
  - Echo Client 本地实现

