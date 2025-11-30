# 为什么使用 reqwest 而不是 core-libs 的 socket/http 模块？

## 问题背景

我们在 `core-libs` 中已经实现了：
- **Socket API** (`core-libs/src/socket/mod.rs`) - 基于 WASI Preview 2 的 socket 抽象
- **IP 池管理** (`core-libs/src/ip/mod.rs`) - IP 地址分配和绑定
- **HTTP 客户端** (`actions-http/src/http_client.rs`) - 基于 socket 的 HTTP 实现

那么为什么在 `HttpActionComponent` 中选择使用 `reqwest` 而不是我们自己的实现？

## 架构对比

### 当前实现方式

```
┌─────────────────────────────────────────────────────────┐
│              Scheduler (Native Binary)                  │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  HttpActionComponent (actions-http)                    │
│         ↓                                               │
│     reqwest::blocking::Client                          │
│         ↓                                               │
│     Rust std::net + rustls                             │
│         ↓                                               │
│     OS Network Stack                                    │
└─────────────────────────────────────────────────────────┘
```

### 如果使用 core-libs 的方案

```
┌─────────────────────────────────────────────────────────┐
│              Scheduler (Native Binary)                  │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  HttpActionComponent (actions-http)                    │
│         ↓                                               │
│     http_client::HttpRequest/Response                  │
│         ↓                                               │
│     core-libs::socket API                              │
│         ↓                                               │
│     WASI Socket Imports ❌ (仅在 WASM 中可用)           │
│         ↓                                               │
│     WASM Runtime + wasi-sockets                        │
└─────────────────────────────────────────────────────────┘
```

## 核心问题分析

### 1. **运行环境差异**

**Scheduler 是 Native 二进制程序，不是 WASM**

```rust
// core-libs/src/socket/mod.rs
pub fn create_socket(...) -> Result<SocketHandle, SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        wasi_impl::create_socket(family, protocol)  // ✅ WASM 中可用
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // ❌ Native 环境：仅是 stub，无实际功能
        let mut registry = SOCKET_REGISTRY.lock().unwrap();
        let info = SocketInfo {
            family,
            protocol,
            connected: false,
            bound: false,
            listening: false,
        };
        Ok(registry.register(info))
    }
}
```

**问题：**
- `core-libs::socket` 的真实实现依赖 WASI socket imports
- 在 Native 环境中，这些 imports 不存在
- 当前的 `#[cfg(not(target_arch = "wasm32"))]` 分支只是占位符

### 2. **HTTP 实现的复杂性**

我们的 `http_client.rs` 实现了基础的 HTTP/1.1：

```rust
// actions-http/src/http_client.rs
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}
```

**缺少的关键功能：**
- ❌ TLS/HTTPS 支持（需要完整的 TLS 实现）
- ❌ HTTP/2 或 HTTP/3
- ❌ 重定向处理
- ❌ Cookie 管理
- ❌ 压缩（gzip, deflate, br）
- ❌ 连接池和复用
- ❌ 超时管理
- ❌ 代理支持
- ❌ 完整的 chunked encoding
- ❌ Keep-alive 优化

### 3. **TLS 是最大的障碍**

真实的 HTTPS 需要：

```rust
// 完整的 TLS 实现需要
1. TLS 握手协议
2. 证书验证（X.509）
3. 加密套件协商
4. 对称加密（AES-GCM, ChaCha20）
5. 非对称加密（RSA, ECDSA）
6. 哈希函数（SHA-256, SHA-384）
7. OCSP/CRL 证书撤销检查
8. SNI (Server Name Indication)
9. ALPN 协商
10. Session resumption
```

**工作量：**
- 自己实现 TLS = 几个月的开发 + 安全审计
- 使用 `rustls` 库 = 开箱即用，经过实战验证

## 设计目标对比

### Core-libs Socket/HTTP 的设计目标

**主要用于 WASM 组件环境：**

```yaml
# 典型的 WASM 组件场景
Component: actions-http (WASM)
  ├─ 运行在 wasmtime runtime
  ├─ 使用 WASI Preview 2 imports
  ├─ 通过 socket imports 访问网络
  ├─ 需要 IP 绑定支持
  └─ 完全沙箱化执行
```

**核心价值：**
1. ✅ **IP 绑定能力** - 通过 WASI socket API 实现源 IP 绑定
2. ✅ **沙箱安全** - WASM 隔离环境
3. ✅ **可移植性** - 跨平台 WASM 组件
4. ✅ **资源控制** - Runtime 级别的资源限制

### Reqwest 的设计目标

**主要用于 Native 应用：**

```rust
// 生产级 HTTP 客户端
use reqwest::blocking::Client;

let client = Client::builder()
    .timeout(Duration::from_secs(30))
    .build()?;

let response = client.get("https://api.example.com")
    .header("Authorization", "Bearer token")
    .send()?;
```

**核心价值：**
1. ✅ **生产就绪** - 数万项目使用，久经考验
2. ✅ **功能完整** - HTTP/1.1, HTTP/2, HTTPS, 压缩等
3. ✅ **性能优化** - 连接池、keep-alive、零拷贝
4. ✅ **易用性** - 简洁的 API，丰富的文档

## 当前架构的合理性

### Scheduler 的角色

Scheduler 是**协调器和测试框架**，不是 WASM 组件：

```
┌──────────────────────────────────────────────────┐
│                  Scheduler                       │
│  (Native Binary - 负载测试框架)                  │
├──────────────────────────────────────────────────┤
│                                                  │
│  ┌──────────────────────────────────────────┐  │
│  │     IP Pool Manager                      │  │
│  │     - 管理 IP 资源池                     │  │
│  │     - 分配 IP 给用户                     │  │
│  └──────────────────────────────────────────┘  │
│                                                  │
│  ┌──────────────────────────────────────────┐  │
│  │     User Executor (并发管理)            │  │
│  │     - 创建用户任务                       │  │
│  │     - 管理生命周期                       │  │
│  │     - 收集统计数据                       │  │
│  └──────────────────────────────────────────┘  │
│                                                  │
│  ┌──────────────────────────────────────────┐  │
│  │     HttpActionComponent (reqwest)        │  │
│  │     - 执行 HTTP 请求                     │  │
│  │     - 记录延迟统计                       │  │
│  └──────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

### WASM Component 的角色

当构建为 WASM 组件时，使用 core-libs 的 socket：

```
┌──────────────────────────────────────────────────┐
│         Actions-HTTP (WASM Component)            │
├──────────────────────────────────────────────────┤
│                                                  │
│  ┌──────────────────────────────────────────┐  │
│  │   http_client (基于 socket API)         │  │
│  │   - 使用 WASI socket imports            │  │
│  │   - 支持 IP 绑定                        │  │
│  └──────────────────────────────────────────┘  │
│            ↓                                     │
│  ┌──────────────────────────────────────────┐  │
│  │   core-libs::socket                     │  │
│  │   - WASI Preview 2 socket API           │  │
│  └──────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
                 ↓
        WASI Runtime (wasmtime)
                 ↓
          真实的 socket 操作
```

## 正确的使用场景

### ✅ 使用 reqwest（当前 Scheduler）

**场景：** Native 二进制程序，需要发起 HTTP 请求

**优势：**
- 完整的 HTTPS 支持
- 生产级性能和稳定性
- 维护成本低
- 丰富的生态系统

**劣势：**
- 无法直接绑定源 IP（需要系统权限）
- 不在 WASM 沙箱中

```rust
// Scheduler/main.rs
let mut component = HttpActionComponent::new(); // 使用 reqwest
executor.run(&mut component)?;
```

### ✅ 使用 core-libs socket（WASM 组件）

**场景：** 构建 WASM 组件，需要 IP 绑定能力

**优势：**
- 可以绑定源 IP（通过 WASI socket API）
- 沙箱安全
- 与 core-libs 生态集成
- 支持自定义网络配置

**劣势：**
- 仅在 WASM 环境可用
- 需要实现完整的 HTTP/TLS 栈
- 开发和维护成本高

```rust
// actions-http/component.rs (WASM target)
#[cfg(target_arch = "wasm32")]
use crate::http_client::{HttpRequest, HttpResponse};

pub fn do_http_request(url: &str) -> Result<HttpResponse> {
    let socket = scheduler_core::socket::create_socket(...)?;
    scheduler_core::socket::bind(socket, bind_ip)?; // ✅ 绑定源 IP
    // ... 使用 socket 发送 HTTP 请求
}
```

## 未来改进方向

### 方案 1: 扩展 reqwest 支持 IP 绑定

**可行性：** 中等
**工作量：** 2-3 周

```rust
// 需要 fork reqwest 或使用底层 hyper
use hyper::client::HttpConnector;

struct IpBindingConnector {
    bind_addr: IpAddr,
}

impl Service<Uri> for IpBindingConnector {
    fn call(&mut self, uri: Uri) -> Self::Future {
        let socket = std::net::TcpSocket::new_v4()?;
        socket.bind(SocketAddr::new(self.bind_addr, 0))?; // 绑定源 IP
        socket.connect(target_addr)?
        // ...
    }
}
```

**限制：** 需要 CAP_NET_RAW 权限或 root 权限

### 方案 2: 完善 core-libs 的 Native 实现

**可行性：** 高
**工作量：** 4-6 周

```rust
// core-libs/src/socket/native_impl.rs
#[cfg(not(target_arch = "wasm32"))]
pub fn create_socket(family: AddressFamily, protocol: SocketProtocol) 
    -> Result<SocketHandle, SocketError> 
{
    // 使用 std::net 或 socket2 实现真实的 socket 操作
    let socket = match protocol {
        SocketProtocol::Tcp => TcpSocket::new_v4()?,
        SocketProtocol::Udp => UdpSocket::new_v4()?,
    };
    
    // 注册到全局 registry
    let handle = register_native_socket(socket);
    Ok(handle)
}
```

**然后在 http_client 中使用：**
```rust
// actions-http/src/http_client.rs
pub fn send_request(req: &HttpRequest, bind_ip: Option<IpAddr>) 
    -> Result<HttpResponse> 
{
    let socket = scheduler_core::socket::create_socket(
        AddressFamily::Ipv4, 
        SocketProtocol::Tcp
    )?;
    
    if let Some(ip) = bind_ip {
        scheduler_core::socket::bind(socket, SocketAddress::new(ip, 0))?;
    }
    
    scheduler_core::socket::connect(socket, target_addr)?;
    
    // 手动实现 HTTP/1.1
    let request_bytes = req.build_request_bytes()?;
    scheduler_core::socket::send(socket, &request_bytes)?;
    
    let response_bytes = scheduler_core::socket::receive(socket, 65536)?;
    HttpResponse::parse(&response_bytes)
}
```

**仍然缺少：**
- ❌ TLS 支持（需要集成 rustls + 手动处理）
- ❌ HTTP/2
- ❌ 连接池
- ❌ 复杂的错误处理

### 方案 3: 混合方案（推荐）

**Native Scheduler:** 使用 reqwest（当前方案）
**WASM Components:** 使用 core-libs socket + http_client

```rust
// actions-http/src/lib.rs
pub struct HttpActionComponent {
    #[cfg(not(target_arch = "wasm32"))]
    client: reqwest::blocking::Client,
    
    #[cfg(target_arch = "wasm32")]
    client: crate::http_client::WasmHttpClient,
}

impl ActionComponent for HttpActionComponent {
    fn do_action(&mut self, action: &ActionDef, ctx: &mut ActionContext) 
        -> Result<ActionOutcome> 
    {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.do_native_request(action, ctx)
        }
        
        #[cfg(target_arch = "wasm32")]
        {
            self.do_wasm_request(action, ctx)
        }
    }
}
```

## 总结

### 当前使用 reqwest 的原因

1. **Scheduler 是 Native 程序**
   - `core-libs::socket` 的实现是为 WASM 设计的
   - Native 分支只是占位符，无实际功能

2. **HTTPS 的复杂性**
   - 实现完整的 TLS 需要大量工作
   - reqwest + rustls 提供了生产级实现

3. **功能完整性**
   - reqwest 提供 HTTP/1.1, HTTP/2, 连接池等
   - 自己实现需要几个月开发

4. **维护成本**
   - reqwest 是成熟的开源项目，持续更新
   - 自己维护 HTTP/TLS 栈成本高

### Core-libs 的价值

Core-libs 的 socket/IP 模块**不是无用的**，它们的价值在于：

1. **WASM 组件环境**
   - 提供 IP 绑定能力
   - 完全控制网络行为
   - 沙箱安全

2. **未来扩展**
   - 可以实现 Native 支持
   - 统一 WASM 和 Native 的 API
   - 自定义网络栈

3. **特殊场景**
   - 需要源 IP 绑定
   - 自定义协议
   - 精细的资源控制

### 最佳实践

**当前架构是合理的：**
- ✅ Scheduler (Native) → reqwest
- ✅ WASM Components → core-libs socket
- ✅ 混合方案，各取所长

**如果需要 IP 绑定：**
1. 短期：在日志中记录 bind_ip（当前方案）
2. 中期：扩展 reqwest 支持 IP 绑定
3. 长期：完善 core-libs 的 Native 实现

**代码复用：**
- 统一的 DSL 和配置（core-libs）
- 统一的执行器接口（executor）
- 不同环境使用不同的底层实现
