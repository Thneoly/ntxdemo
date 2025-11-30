# Actions-HTTP 使用 Core-Libs Socket 的架构设计

## 概述

actions-http 组件通过 core-libs 组件的 socket API 实现 HTTP 请求，支持 IP 池管理和自定义网络配置。

## 架构方案

由于 WASM 组件模型的限制，我们采用以下架构：

### 方案 1: 组合模式 (Composition - 推荐)

两个独立的 WASM 组件通过 WAC (WebAssembly Composition) 组合：

```
┌─────────────────────────────────────┐
│   Composed Component                │
│                                     │
│  ┌──────────────┐  ┌──────────────┐│
│  │  core-libs   │  │ actions-http ││
│  │              │  │              ││
│  │  - Socket    │──▶│ HTTP Client  ││
│  │  - IP Pool   │  │              ││
│  └──────────────┘  └──────────────┘│
└─────────────────────────────────────┘
```

**优点**:
- 组件职责清晰分离
- core-libs 可被多个组件复用
- 符合 WASM 组件模型最佳实践

**实现步骤**:

1. **构建 core-libs 组件** (wasm32-wasip1 或 wasip2)
   ```bash
   cd plugins/scheduler/core-libs
   cargo component build --release
   ```

2. **构建 actions-http 组件**
   ```bash
   cd plugins/scheduler/actions-http
   cargo component build --release
   ```

3. **使用 WAC 组合**
   ```bash
   wac compose http-with-socket.wac \
     --component core-libs=../core-libs/target/wasm32-wasip1/release/scheduler_core.wasm \
     --component actions-http=target/wasm32-wasip2/release/scheduler_actions_http.wasm \
     -o composed-http-action.wasm
   ```

4. **在 Executor/Scheduler 中使用组合后的组件**

### 方案 2: 静态链接模式

在 actions-http 的 Rust 代码中直接调用 scheduler-core 库（非 WASM 导入）：

**优点**:
- 单个 WASM 文件，部署简单
- 不需要额外的组合工具

**缺点**:
- 代码耦合度高
- 无法独立更新 core-libs

**当前实现**:
- `src/http_client.rs` - HTTP 请求/响应解析
- `src/component.rs` - 使用 WIT bindings 调用 socket

### 方案 3: 运行时链接模式

通过 Executor/Scheduler 提供统一的 socket 接口：

```
┌────────────────────────┐
│   Scheduler/Executor   │
│                        │
│  - Instance core-libs  │
│  - Instance actions-http│
│  - Link at runtime     │
└────────────────────────┘
```

**优点**:
- 灵活的运行时配置
- 支持动态组件加载

**缺点**:
- Executor 需要实现组件间调用

## 当前实现状态

### 已完成

1. ✅ **WIT 接口定义**
   - `actions-http/wit/world.wit` 定义了导入接口
   - `actions-http/wit/deps.toml` 声明 core-libs 依赖

2. ✅ **HTTP 客户端实现**
   - `src/http_client.rs` - 基于 socket 的 HTTP 请求/响应
   - 支持 HTTP/1.1 协议
   - 简单的 URL 解析和 header 处理

3. ✅ **Component Bindings**
   - `src/component.rs` - WIT 导出实现
   - 使用 `scheduler::core_libs::socket` 进行网络通信
   - DNS 解析占位符（需要增强）

### 待完成

1. ⏳ **解决组件构建依赖**
   - 当前问题: cargo-component 找不到 scheduler:core-libs
   - 解决方案: 需要先构建 core-libs 并配置正确的依赖路径

2. ⏳ **实现 DNS 解析**
   - 当前: 仅支持 IP 地址和 localhost
   - 需要: 集成 WASI DNS 或实现简单的 hosts 映射

3. ⏳ **HTTPS 支持**
   - 需要 TLS 库（如 rustls）
   - 或使用 WASI crypto 接口

4. ⏳ **IP 池集成**
   - 支持从 core-libs 的 IP 池分配 IP
   - 将 socket 绑定到特定 IP

5. ⏳ **错误处理增强**
   - 更详细的网络错误映射
   - 超时处理
   - 重试机制

## 构建说明

### 前置条件

```bash
# 安装 cargo-component
cargo install cargo-component

# 安装 wac (WebAssembly Composition tool)
cargo install wac-cli
```

### 构建步骤

#### 步骤 1: 构建 core-libs (wasip1)

```bash
cd plugins/scheduler/core-libs
cargo clean
cargo component build --release --target wasm32-wasip1

# 验证输出
ls -lh target/wasm32-wasip1/release/scheduler_core.wasm
```

#### 步骤 2: 修复 actions-http 依赖

Option A - 使用预构建组件:
```bash
# 将 core-libs 组件复制到 actions-http 可访问的位置
mkdir -p plugins/scheduler/actions-http/deps
cp plugins/scheduler/core-libs/target/wasm32-wasip1/release/scheduler_core.wasm \
   plugins/scheduler/actions-http/deps/
```

Option B - 暂时移除 WIT 导入，使用 Rust 依赖:
```wit
// 注释掉 world.wit 中的导入
// import scheduler:core-libs/socket@0.1.0;
```

#### 步骤 3: 构建 actions-http

```bash
cd plugins/scheduler/actions-http
cargo component build --release --target wasm32-wasip2

# 验证输出
ls -lh target/wasm32-wasip2/release/scheduler_actions_http.wasm
```

#### 步骤 4: 组合组件 (可选)

```bash
cd plugins/scheduler/composed
wac compose http-with-socket.wac \
  --component core-libs=../core-libs/target/wasm32-wasip1/release/scheduler_core.wasm \
  --component actions-http=../actions-http/target/wasm32-wasip2/release/scheduler_actions_http.wasm \
  -o http-with-socket-composed.wasm

# 检查组合后的组件
wasm-tools component wit http-with-socket-composed.wasm
```

## 使用示例

### DSL Action 定义

```yaml
actions:
  - id: http-get-example
    call: GET
    with:
      url: "http://192.168.1.100:8080/api/data"
      headers:
        User-Agent: "Scheduler-HTTP-Client"
        Accept: "application/json"
    export:
      - type: variable
        name: response_body
        scope: global

  - id: http-post-with-ip
    call: POST
    with:
      url: "http://example.com/api/submit"
      bind_ip: "10.0.0.5"  # 使用特定 IP (需要 IP 池集成)
      headers:
        Content-Type: "application/json"
      body:
        data: "test"
        timestamp: "{{now}}"
```

### 在 Executor 中使用

```rust
// 加载组合后的组件
let component = load_component("http-with-socket-composed.wasm")?;

// 执行 HTTP action
let action = ActionDef {
    id: "test-http".to_string(),
    call: "GET".to_string(),
    with: hashmap! {
        "url" => "http://192.168.1.1/test"
    },
    export: vec![],
};

let outcome = component.do_http_action(&action)?;
println!("HTTP result: {:?}", outcome);
```

## 下一步计划

1. **解决构建依赖** - 配置正确的组件依赖路径或使用组合模式
2. **增强 DNS** - 实现真实的域名解析
3. **IP 池集成** - 连接 core-libs 的 IP 分配功能
4. **测试** - 编写端到端测试
5. **文档** - 补充 API 文档和使用示例

## 参考

- [WASM Component Model](https://github.com/WebAssembly/component-model)
- [cargo-component](https://github.com/bytecodealliance/cargo-component)
- [WAC - WebAssembly Composition](https://github.com/bytecodealliance/wac)
- [WASI Preview 2](https://github.com/WebAssembly/WASI)
