# Socket API 使用指南

## 概述

core-libs 组件现在提供了 WASI socket 的封装接口，支持 TCP 和 UDP 网络通信。这些接口可供所有业务组件（如 actions-http）使用。

## Socket 接口

### 数据类型

#### AddressFamily
```wit
enum address-family {
    ipv4,  // IPv4 地址族
    ipv6,  // IPv6 地址族
}
```

#### SocketProtocol
```wit
enum socket-protocol {
    tcp,  // TCP 协议
    udp,  // UDP 协议
}
```

#### SocketAddress
```wit
record socket-address {
    host: string,  // 主机名或 IP 地址
    port: u16,     // 端口号
}
```

#### SocketError
```wit
enum socket-error {
    connection-refused,       // 连接被拒绝
    connection-reset,         // 连接被重置
    connection-aborted,       // 连接被中止
    network-unreachable,      // 网络不可达
    address-in-use,          // 地址已被使用
    address-not-available,   // 地址不可用
    timeout,                 // 超时
    would-block,             // 非阻塞操作会阻塞
    invalid-input,           // 无效输入
    other,                   // 其他错误
}
```

### 核心函数

#### 1. 创建 Socket
```wit
create-socket: func(
    family: address-family,
    protocol: socket-protocol
) -> result<socket-handle, socket-error>
```

**示例（TCP IPv4）:**
```rust
use scheduler_core::component::exports::scheduler::core_libs::socket;

let socket_handle = socket::create_socket(
    socket::AddressFamily::Ipv4,
    socket::SocketProtocol::Tcp
)?;
```

#### 2. 连接到远程地址（TCP）
```wit
connect: func(
    socket: socket-handle,
    address: socket-address
) -> result<_, socket-error>
```

**示例:**
```rust
let addr = socket::SocketAddress {
    host: "example.com".to_string(),
    port: 80,
};
socket::connect(socket_handle, addr)?;
```

#### 3. 绑定到本地地址
```wit
bind: func(
    socket: socket-handle,
    address: socket-address
) -> result<_, socket-error>
```

**示例:**
```rust
let addr = socket::SocketAddress {
    host: "0.0.0.0".to_string(),
    port: 8080,
};
socket::bind(socket_handle, addr)?;
```

#### 4. 监听连接（TCP 服务器）
```wit
listen: func(
    socket: socket-handle,
    backlog: u32
) -> result<_, socket-error>
```

**示例:**
```rust
socket::listen(socket_handle, 128)?;
```

#### 5. 接受连接（TCP 服务器）
```wit
accept: func(
    socket: socket-handle
) -> result<socket-handle, socket-error>
```

**示例:**
```rust
let client_socket = socket::accept(server_socket)?;
```

#### 6. 发送数据
```wit
send: func(
    socket: socket-handle,
    data: list<u8>
) -> result<u64, socket-error>
```

**示例:**
```rust
let data = b"Hello, World!";
let bytes_sent = socket::send(socket_handle, data.to_vec())?;
```

#### 7. 接收数据
```wit
receive: func(
    socket: socket-handle,
    max-len: u64
) -> result<list<u8>, socket-error>
```

**示例:**
```rust
let data = socket::receive(socket_handle, 4096)?;
let response = String::from_utf8_lossy(&data);
```

#### 8. 发送数据到指定地址（UDP）
```wit
send-to: func(
    socket: socket-handle,
    data: list<u8>,
    address: socket-address
) -> result<u64, socket-error>
```

**示例:**
```rust
let addr = socket::SocketAddress {
    host: "192.168.1.100".to_string(),
    port: 5000,
};
let bytes_sent = socket::send_to(socket_handle, data, addr)?;
```

#### 9. 从指定地址接收数据（UDP）
```wit
receive-from: func(
    socket: socket-handle,
    max-len: u64
) -> result<tuple<list<u8>, socket-address>, socket-error>
```

**示例:**
```rust
let (data, sender_addr) = socket::receive_from(socket_handle, 4096)?;
println!("Received from {}:{}", sender_addr.host, sender_addr.port);
```

#### 10. 关闭 Socket
```wit
close: func(socket: socket-handle) -> result<_, socket-error>
```

**示例:**
```rust
socket::close(socket_handle)?;
```

### Socket 选项

#### 设置读超时
```wit
set-read-timeout: func(
    socket: socket-handle,
    timeout-ms: option<u64>
) -> result<_, socket-error>
```

**示例:**
```rust
// 设置 5 秒超时
socket::set_read_timeout(socket_handle, Some(5000))?;

// 取消超时（阻塞模式）
socket::set_read_timeout(socket_handle, None)?;
```

#### 设置写超时
```wit
set-write-timeout: func(
    socket: socket-handle,
    timeout-ms: option<u64>
) -> result<_, socket-error>
```

#### 设置地址重用
```wit
set-reuse-address: func(
    socket: socket-handle,
    reuse: bool
) -> result<_, socket-error>
```

**示例:**
```rust
socket::set_reuse_address(socket_handle, true)?;
```

#### 获取本地地址
```wit
get-local-address: func(
    socket: socket-handle
) -> result<socket-address, socket-error>
```

#### 获取对端地址
```wit
get-peer-address: func(
    socket: socket-handle
) -> result<socket-address, socket-error>
```

## 使用场景

### 场景 1: TCP 客户端

```rust
use scheduler_core::component::exports::scheduler::core_libs::socket;

// 1. 创建 socket
let socket = socket::create_socket(
    socket::AddressFamily::Ipv4,
    socket::SocketProtocol::Tcp
)?;

// 2. 设置超时
socket::set_read_timeout(socket, Some(10000))?;
socket::set_write_timeout(socket, Some(10000))?;

// 3. 连接到服务器
let addr = socket::SocketAddress {
    host: "api.example.com".to_string(),
    port: 443,
};
socket::connect(socket, addr)?;

// 4. 发送请求
let request = b"GET / HTTP/1.1\r\nHost: api.example.com\r\n\r\n";
socket::send(socket, request.to_vec())?;

// 5. 接收响应
let response = socket::receive(socket, 8192)?;

// 6. 关闭连接
socket::close(socket)?;
```

### 场景 2: TCP 服务器

```rust
use scheduler_core::component::exports::scheduler::core_libs::socket;

// 1. 创建监听 socket
let server = socket::create_socket(
    socket::AddressFamily::Ipv4,
    socket::SocketProtocol::Tcp
)?;

// 2. 设置选项
socket::set_reuse_address(server, true)?;

// 3. 绑定地址
let addr = socket::SocketAddress {
    host: "0.0.0.0".to_string(),
    port: 8080,
};
socket::bind(server, addr)?;

// 4. 开始监听
socket::listen(server, 128)?;

// 5. 接受连接（通常在循环中）
let client = socket::accept(server)?;

// 6. 处理客户端请求
let data = socket::receive(client, 4096)?;
let response = b"HTTP/1.1 200 OK\r\n\r\nHello!";
socket::send(client, response.to_vec())?;

// 7. 关闭连接
socket::close(client)?;
socket::close(server)?;
```

### 场景 3: UDP 通信

```rust
use scheduler_core::component::exports::scheduler::core_libs::socket;

// 1. 创建 UDP socket
let socket = socket::create_socket(
    socket::AddressFamily::Ipv4,
    socket::SocketProtocol::Udp
)?;

// 2. 绑定本地地址（可选）
let local_addr = socket::SocketAddress {
    host: "0.0.0.0".to_string(),
    port: 5000,
};
socket::bind(socket, local_addr)?;

// 3. 发送数据
let remote_addr = socket::SocketAddress {
    host: "192.168.1.100".to_string(),
    port: 5001,
};
let data = b"Hello UDP";
socket::send_to(socket, data.to_vec(), remote_addr)?;

// 4. 接收数据
let (response, sender) = socket::receive_from(socket, 4096)?;
println!("Got response from {}:{}", sender.host, sender.port);

// 5. 关闭 socket
socket::close(socket)?;
```

## 在 actions-http 中使用

在 actions-http 组件中可以直接使用 socket 接口实现 HTTP 请求：

```rust
// actions-http/src/component.rs
use scheduler_core::component::exports::scheduler::core_libs::socket;

fn http_request(url: &str) -> Result<String, String> {
    // 解析 URL
    let (host, port, path) = parse_url(url)?;
    
    // 创建 socket
    let socket = socket::create_socket(
        socket::AddressFamily::Ipv4,
        socket::SocketProtocol::Tcp
    ).map_err(|e| format!("Failed to create socket: {:?}", e))?;
    
    // 设置超时
    socket::set_read_timeout(socket, Some(30000))
        .map_err(|e| format!("Failed to set timeout: {:?}", e))?;
    
    // 连接
    let addr = socket::SocketAddress {
        host: host.to_string(),
        port,
    };
    socket::connect(socket, addr)
        .map_err(|e| format!("Failed to connect: {:?}", e))?;
    
    // 发送 HTTP 请求
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, host
    );
    socket::send(socket, request.as_bytes().to_vec())
        .map_err(|e| format!("Failed to send: {:?}", e))?;
    
    // 接收响应
    let mut response = Vec::new();
    loop {
        match socket::receive(socket, 4096) {
            Ok(data) if data.is_empty() => break,
            Ok(data) => response.extend_from_slice(&data),
            Err(socket::SocketError::WouldBlock) => break,
            Err(e) => return Err(format!("Failed to receive: {:?}", e)),
        }
    }
    
    // 关闭连接
    socket::close(socket).ok();
    
    String::from_utf8(response)
        .map_err(|e| format!("Invalid UTF-8: {}", e))
}
```

## 错误处理

建议使用模式匹配处理不同的错误类型：

```rust
match socket::connect(socket, addr) {
    Ok(_) => {
        // 连接成功
    }
    Err(socket::SocketError::ConnectionRefused) => {
        // 连接被拒绝，可能服务器未运行
    }
    Err(socket::SocketError::Timeout) => {
        // 连接超时
    }
    Err(socket::SocketError::NetworkUnreachable) => {
        // 网络不可达
    }
    Err(e) => {
        // 其他错误
        return Err(format!("Connection failed: {:?}", e));
    }
}
```

## 注意事项

1. **资源管理**: 始终记得在使用完 socket 后调用 `close()` 释放资源
2. **超时设置**: 建议为网络操作设置合理的超时时间，避免无限等待
3. **错误处理**: 网络操作可能失败，务必妥善处理各种错误情况
4. **IPv6 支持**: 如需使用 IPv6，创建 socket 时指定 `AddressFamily::Ipv6`
5. **UDP 特性**: UDP 是无连接协议，不需要 connect/listen/accept 操作
6. **地址重用**: 服务器程序建议启用 `set-reuse-address`，避免重启时端口被占用

## 实现状态

当前实现状态：
- ✅ WIT 接口定义完整
- ✅ Rust 内部实现（stub 版本，用于 WASM 环境测试）
- ✅ 组件绑定生成
- ✅ 导出到所有业务组件
- 🚧 真实 WASI socket 集成（待实现）

下一步工作：
1. 集成实际的 WASI socket API（wasi:sockets/tcp, wasi:sockets/udp）
2. 实现完整的错误映射
3. 添加异步 socket 支持
4. 性能优化和测试

---
文档日期: 2025-11-30
版本: 0.1.0
