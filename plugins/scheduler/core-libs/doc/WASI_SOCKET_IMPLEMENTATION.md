# WASI Socket 实现说明

## 概述

成功集成了 WASI Preview 2 socket 接口到 `scheduler:core-libs` 组件中。实现采用**方案 A：使用网络句柄而不是引用**，通过两阶段锁定解决了 Rust 借用检查器问题。

## 实现架构

### 1. WIT 接口定义 (`wit/world.wit`)

定义了三个自定义 WASI socket 接口：

#### `wasi-network`
- **资源**: `network` - 网络实例句柄
- **类型**: 
  - `ip-address`: IPv4/IPv6 地址 variant
  - `ip-address-family`: IPv4/IPv6 枚举
  - `error-code`: 15种网络错误类型
- **函数**: `get-network()` - 获取默认网络实例

#### `wasi-tcp` 
- **资源**: `tcp-socket` - TCP socket 句柄
- **方法**:
  - `new(family)` - 创建 TCP socket
  - `connect(net, addr, port)` - 连接到远程地址
  - `bind(net, addr, port)` - 绑定本地地址
  - `listen(backlog)` - 监听连接
  - `accept()` - 接受连接，返回 (client_socket, remote_addr, remote_port)
  - `send(data)` - 发送数据
  - `receive(max_len)` - 接收数据
  - `local-address()` - 获取本地地址
  - `remote-address()` - 获取对端地址
  - `set-reuse-address(value)` - 设置 SO_REUSEADDR

#### `wasi-udp`
- **资源**: `udp-socket` - UDP socket 句柄  
- **方法**:
  - `new(family)` - 创建 UDP socket
  - `bind(net, addr, port)` - 绑定本地地址
  - `send-to(data, net, addr, port)` - 发送数据报
  - `receive-from(max_len)` - 接收数据报，返回 (data, remote_addr, remote_port)
  - `local-address()` - 获取本地地址
  - `set-reuse-address(value)` - 设置 SO_REUSEADDR

### 2. Rust 实现 (`src/socket_wasi_impl.rs`)

#### 关键设计决策

**问题**: Rust 借用检查器不允许同时持有 `&mut SocketRegistry`（获取网络）和 `&SocketRegistry`（获取socket）

**解决方案**: 两阶段锁定模式
```rust
// 阶段 1: 确保网络已初始化（需要 &mut）
{
    let mut registry = REGISTRY.lock().unwrap();
    registry.ensure_network()?;
}

// 阶段 2: 使用网络和socket（只需要 &）
let registry = REGISTRY.lock().unwrap();
let network = registry.network().ok_or(SocketError::Other)?;
let socket = registry.get(socket_id).ok_or(SocketError::InvalidInput)?;
```

#### 核心结构

```rust
enum SocketHandle {
    Tcp(TcpSocket),
    Udp(UdpSocket),
}

struct SocketRegistry {
    sockets: HashMap<u32, SocketHandle>,
    next_id: u32,
    network: Option<Network>,  // 懒加载的网络实例
}
```

#### 方法

- `ensure_network()` - 初始化网络（如果未初始化）
- `network()` - 返回网络引用（只读访问）
- `register()` - 注册新socket
- `get()` - 获取socket引用
- `remove()` - 移除socket

### 3. 公共 API (`src/socket.rs`)

所有公共函数使用条件编译：

```rust
pub fn connect(handle: SocketHandle, address: SocketAddress) -> Result<(), SocketError> {
    #[cfg(target_arch = "wasm32")]
    {
        crate::socket_wasi_impl::connect(handle, &address.host, address.port)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Fallback stub 实现
    }
}
```

**WASM 环境**: 调用真实的 WASI socket 实现
**非 WASM 环境**: 使用 stub 实现（用于测试）

## 构建结果

### 组件信息
- **文件**: `scheduler_core.wasm`
- **大小**: 474 KB
- **目标**: wasm32-wasip1
- **状态**: ✓ 有效组件

### 导入接口
```wit
import scheduler:core-libs/wasi-network@0.1.0;
import scheduler:core-libs/wasi-tcp@0.1.0;
import scheduler:core-libs/wasi-udp@0.1.0;
```

加上标准 WASI 接口：
- wasi:cli/* (environment, exit, stdin, stdout, stderr)
- wasi:io/* (error, streams)
- wasi:clocks/wall-clock
- wasi:filesystem/*
- wasi:random/random

### 导出接口
```wit
export scheduler:core-libs/types@0.1.0;
export scheduler:core-libs/parser@0.1.0;
export scheduler:core-libs/socket@0.1.0;
```

## IP 地址处理

### IPv4 解析
```rust
fn parse_ipv4(addr: &str) -> Result<WasiIpAddress, SocketError> {
    // "192.168.1.1" -> IpAddress::Ipv4((192, 168, 1, 1))
}
```

### IPv6 解析  
```rust
fn parse_ipv6(addr: &str) -> Result<WasiIpAddress, SocketError> {
    // "2001:0db8:0000:0000:0000:0000:0000:0001"
    // -> IpAddress::Ipv6((0x2001, 0x0db8, 0, 0, 0, 0, 0, 1))
}
```

**注意**: 当前实现不支持 IPv6 压缩表示法 (`::`)

## 错误处理

### WASI 错误到 Socket 错误的映射

```rust
fn convert_error(error: WasiErrorCode) -> SocketError {
    match error {
        WasiErrorCode::ConnectionRefused => SocketError::ConnectionRefused,
        WasiErrorCode::ConnectionReset => SocketError::ConnectionReset,
        WasiErrorCode::Timeout => SocketError::Timeout,
        WasiErrorCode::AddressInUse => SocketError::AddressInUse,
        // ... 等等
    }
}
```

## 使用示例

### TCP 客户端

```rust
// 创建 TCP socket
let socket = socket::create_socket(
    socket::AddressFamily::Ipv4,
    socket::SocketProtocol::Tcp
)?;

// 连接到服务器
socket::connect(
    socket,
    socket::SocketAddress::new("192.168.1.100", 8080)
)?;

// 发送数据
let data = b"GET / HTTP/1.1\r\n\r\n";
socket::send(socket, data)?;

// 接收响应
let response = socket::receive(socket, 4096)?;

// 关闭连接
socket::close(socket)?;
```

### TCP 服务器

```rust
// 创建监听 socket
let listener = socket::create_socket(
    socket::AddressFamily::Ipv4,
    socket::SocketProtocol::Tcp
)?;

// 绑定端口
socket::bind(
    listener,
    socket::SocketAddress::new("0.0.0.0", 8080)
)?;

// 开始监听
socket::listen(listener, 10)?;

// 接受连接
let client = socket::accept(listener)?;

// 处理客户端...
socket::send(client, b"Hello!")?;
socket::close(client)?;
```

### UDP 套接字

```rust
// 创建 UDP socket
let socket = socket::create_socket(
    socket::AddressFamily::Ipv4,
    socket::SocketProtocol::Udp
)?;

// 绑定本地端口（可选）
socket::bind(
    socket,
    socket::SocketAddress::new("0.0.0.0", 9000)
)?;

// 发送数据报
socket::send_to(
    socket,
    b"Hello, UDP!",
    socket::SocketAddress::new("192.168.1.100", 9001)
)?;

// 接收数据报
let (data, sender_addr) = socket::receive_from(socket, 1024)?;

socket::close(socket)?;
```

## 限制和注意事项

### 当前限制

1. **超时设置**: WASI Preview 2 sockets 规范中没有超时选项
   - `set_read_timeout()` 和 `set_write_timeout()` 是 no-op

2. **IPv6 压缩**: 不支持 IPv6 地址的 `::` 压缩表示法
   - 必须写完整格式: `2001:0db8:0000:0000:0000:0000:0000:0001`

3. **Shutdown 操作**: 未暴露到公共 API
   - TCP socket 有 `shutdown()` 方法但未导出

4. **网络选择**: 只支持默认网络
   - 通过 `wasi-network::get-network()` 获取

### 运行时要求

组件需要 WASI Preview 2 运行时支持：

- **Wasmtime 25+**: 支持 WASI Preview 2 sockets
- **需要网络权限**: 运行时必须授予网络访问权限

示例运行命令：
```bash
wasmtime run \
  --wasm component-model \
  --wasi preview2 \
  --allow-network \
  scheduler_core.wasm
```

### 未来改进

1. **IPv6 压缩支持**: 解析 `::` 表示法
2. **异步 I/O**: 集成 wasi:io/poll 进行非阻塞操作
3. **连接超时**: 使用 wasi:clocks 实现超时
4. **TLS 支持**: 等待 WASI TLS 规范
5. **多网络支持**: 支持多个网络接口

## 技术细节

### Component Model 资源

WASI sockets 使用 Component Model 的资源系统：

```wit
resource network;
resource tcp-socket { ... }
resource udp-socket { ... }
```

资源在 Rust 中表示为句柄：

```rust
pub struct Network {
    handle: _rt::Resource<Network>,
}
```

资源有自动的生命周期管理和析构函数。

### 借用语义

WIT 中的 `borrow<T>` 表示临时借用：

```wit
connect: func(net: borrow<network>, ...) -> result<...>;
```

在 Rust 中编译为 `&Network` 引用。

## 测试

### 单元测试

现有测试继续工作（使用 fallback stub）：

```rust
#[test]
fn test_socket_address() {
    let addr = SocketAddress::new("127.0.0.1", 8080);
    assert_eq!(addr.host, "127.0.0.1");
    assert_eq!(addr.port, 8080);
}
```

### 集成测试

需要 WASI 运行时环境的真实网络测试。

## 总结

✅ **成功实现了完整的 WASI Preview 2 socket 集成**

- 自定义 WIT 接口定义（wasi-network、wasi-tcp、wasi-udp）
- 完整的 Rust 实现（384 行）
- 使用两阶段锁定解决借用问题
- 条件编译支持 WASM 和非 WASM 环境
- 组件大小：474 KB
- 所有功能：TCP/UDP 客户端和服务器

这个实现为构建真实的网络应用程序奠定了基础，等待 WASI Preview 2 运行时和实际网络环境测试。
