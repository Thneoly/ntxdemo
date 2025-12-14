# Socket with IP Pool Integration

## 概述

此更新集成了 IP 池管理和 Socket API，提供了完整的网络编程能力，支持将 socket 绑定到特定的 IP 地址进行收发包操作。这为后续开发 HTTP 组件提供了基础。

## 新增功能

### 1. Socket 高级 API (`socket/api.rs`)

提供了一个符合标准 socket 编程模型的高级 API：

```rust
// 标准流程: socket() -> bind() -> listen()/connect() -> send()/recv()

// 创建 socket
let mut sock = Socket::tcp_v4()?;

// 绑定到 IP
sock.bind_to_ip(ip, port)?;

// TCP 服务端: 监听连接
sock.listen(backlog)?;
let client = sock.accept()?;

// TCP 客户端: 连接服务器
sock.connect(server_addr)?;

// 收发数据
sock.send(data)?;
let data = sock.recv(max_len)?;

// 关闭 socket
sock.close()?;
```

### 2. Socket 类型

#### `Socket` 结构体
封装了底层 socket handle，提供状态管理和类型安全的 API。

**字段：**
- `handle`: 底层 socket 句柄
- `protocol`: TCP/UDP
- `family`: IPv4/IPv6
- `bound_ip`: 绑定的 IP 地址
- `bound_port`: 绑定的端口
- `remote_addr`: 远程地址（TCP 连接）
- `state`: Socket 状态（Created/Bound/Listening/Connected/Closed）

**创建方法：**
```rust
Socket::tcp_v4()    // TCP IPv4
Socket::tcp_v6()    // TCP IPv6
Socket::udp_v4()    // UDP IPv4
Socket::udp_v6()    // UDP IPv6
```

## 核心 API

### Socket 创建

```rust
use scheduler_core::Socket;

// TCP socket
let mut tcp_sock = Socket::tcp_v4()?;

// UDP socket
let mut udp_sock = Socket::udp_v4()?;
```

### IP 绑定

#### 方法 1: 直接绑定 IP 地址

```rust
use std::net::IpAddr;

let ip: IpAddr = "192.168.1.10".parse()?;
sock.bind_to_ip(ip, 8080)?;
```

#### 方法 2: 使用 IP 池分配的 IP

```rust
use scheduler_core::{IpPool, ResourceType};

// 从 IP 池分配 IP
let mut pool = IpPool::new("my-pool");
pool.add_cidr_range("192.168.1.0/24")?;

let ip = pool.allocate(
    "instance-01",
    "my-service",
    ResourceType::Custom("socket".into())
)?;

// 绑定 socket 到分配的 IP
sock.bind_to_ip(ip, 8080)?;
```

#### 方法 3: 使用 IpBinding

```rust
// 如果你已经有 IpBinding
let binding = pool.get_binding(&ip).unwrap();
sock.bind_with_binding(binding, 8080)?;
```

### TCP 服务端模式

```rust
// 1. 创建并绑定
let mut server = Socket::tcp_v4()?;
server.bind_to_ip(ip, 8080)?;

// 2. 监听
server.listen(100)?; // backlog = 100

// 3. 接受连接
let mut client = server.accept()?;

// 4. 收发数据
let data = client.recv(1024)?;
client.send(b"Response")?;

// 5. 关闭
client.close()?;
server.close()?;
```

### TCP 客户端模式

```rust
// 1. 创建 socket
let mut client = Socket::tcp_v4()?;

// 2. （可选）绑定源 IP
client.bind_to_ip(local_ip, 0)?; // port 0 = 让系统选择

// 3. 连接服务器
let server_addr = SocketAddress::new("192.168.1.1", 8080);
client.connect(server_addr)?;

// 4. 发送请求
client.send(b"GET / HTTP/1.1\r\n\r\n")?;

// 5. 接收响应
let response = client.recv(4096)?;

// 6. 关闭
client.close()?;
```

### UDP 模式

```rust
// 1. 创建并绑定
let mut udp = Socket::udp_v4()?;
udp.bind_to_ip(ip, 9000)?;

// 2. 发送到特定地址
let target = SocketAddress::new("192.168.1.10", 9001);
udp.send_to(b"Hello", target)?;

// 3. 从任意地址接收
let (data, sender) = udp.recv_from(1024)?;
println!("Received from {}", sender.host);

// 4. 关闭
udp.close()?;
```

## 完整示例

### 示例 1: TCP 服务器与 IP 池集成

```rust
use scheduler_core::{IpPool, ResourceType, Socket};

// 创建 IP 池
let mut pool = IpPool::new("server-pool");
pool.add_cidr_range("192.168.1.0/24")?;

// 分配 IP
let server_ip = pool.allocate(
    "web-services",
    "http-server-1",
    ResourceType::Custom("tcp-server".into())
)?;

// 创建并绑定 socket
let mut server = Socket::tcp_v4()?;
server.bind_to_ip(server_ip, 8080)?;
server.listen(128)?;

println!("Server listening on {}:8080", server_ip);

// 接受连接并处理
loop {
    let mut client = server.accept()?;
    
    // 处理客户端请求
    let request = client.recv(4096)?;
    let response = process_request(&request);
    client.send(&response)?;
    
    client.close()?;
}
```

### 示例 2: 多租户场景

```rust
// 租户 A - Web 服务
let tenant_a_ip = pool.allocate("tenant-a", "web", 
    ResourceType::Vm("web-vm-001".into()))?;
let mut tenant_a_sock = Socket::tcp_v4()?;
tenant_a_sock.bind_to_ip(tenant_a_ip, 80)?;
tenant_a_sock.listen(100)?;

// 租户 B - API 服务
let tenant_b_ip = pool.allocate("tenant-b", "api",
    ResourceType::Container("api-container".into()))?;
let mut tenant_b_sock = Socket::tcp_v4()?;
tenant_b_sock.bind_to_ip(tenant_b_ip, 8080)?;
tenant_b_sock.listen(200)?;

// 查询租户使用情况
let tenant_a_ips = pool.list_by_subinstance("tenant-a");
println!("Tenant A: {} IPs", tenant_a_ips.len());
```

### 示例 3: HTTP 服务基础

```rust
// 为 HTTP 组件奠定基础
let mut pool = IpPool::new("http-pool");
pool.add_cidr_range("192.168.100.0/24")?;

let http_ip = pool.allocate(
    "http-services",
    "http-server",
    ResourceType::Custom("http-server".into())
)?;

// HTTP 服务器 socket
let mut server = Socket::tcp_v4()?;
server.bind_to_ip(http_ip, 80)?;
server.listen(1000)?;

// 接受 HTTP 请求
let mut client = server.accept()?;

// 接收 HTTP 请求
let request_data = client.recv(8192)?;
let request_str = String::from_utf8_lossy(&request_data);

// 解析 HTTP (未来的 HTTP 组件功能)
// let (method, path, headers) = parse_http_request(&request_str);

// 发送 HTTP 响应
let response = b"HTTP/1.1 200 OK\r\n\
Content-Type: text/plain\r\n\
Content-Length: 13\r\n\
\r\n\
Hello, World!";

client.send(response)?;
client.close()?;
```

## API 参考

### Socket 方法

#### 创建
- `Socket::tcp_v4()` - 创建 TCP IPv4 socket
- `Socket::tcp_v6()` - 创建 TCP IPv6 socket
- `Socket::udp_v4()` - 创建 UDP IPv4 socket
- `Socket::udp_v6()` - 创建 UDP IPv6 socket
- `Socket::new_tcp(family)` - 创建 TCP socket（指定地址族）
- `Socket::new_udp(family)` - 创建 UDP socket（指定地址族）

#### 绑定
- `bind_to_ip(ip, port)` - 绑定到 IP 地址和端口
- `bind(addr)` - 绑定到 SocketAddress
- `bind_with_binding(binding, port)` - 使用 IpBinding 绑定

#### TCP 操作
- `listen(backlog)` - 监听连接（TCP 服务端）
- `connect(addr)` - 连接到服务器（TCP 客户端）
- `accept()` - 接受连接（TCP 服务端）

#### 数据传输
- `send(data)` - 发送数据
- `recv(max_len)` - 接收数据
- `send_to(data, addr)` - 发送到指定地址（UDP）
- `recv_from(max_len)` - 从任意地址接收（UDP）

#### 信息查询
- `handle()` - 获取 socket 句柄
- `local_ip()` - 获取本地 IP
- `local_port()` - 获取本地端口
- `remote_addr()` - 获取远程地址
- `protocol()` - 获取协议类型
- `family()` - 获取地址族
- `is_connected()` - 是否已连接
- `is_bound()` - 是否已绑定
- `is_listening()` - 是否在监听

#### 关闭
- `close()` - 关闭 socket

### 与 IP 池集成

```rust
// IP 池 + Socket 完整流程

// 1. 创建 IP 池
let mut pool = IpPool::new("my-pool");
pool.add_cidr_range("10.0.0.0/16")?;

// 2. 分配 IP
let ip = pool.allocate(
    "instance",
    "resource-id",
    ResourceType::Custom("socket".into())
)?;

// 3. 创建 socket
let mut sock = Socket::tcp_v4()?;

// 4. 绑定 IP
sock.bind_to_ip(ip, 8080)?;

// 5. 使用 socket
sock.listen(100)?;

// 6. 清理
sock.close()?;
pool.release_by_subid("instance", "resource-id")?;
```

## 状态机

Socket 的状态转换：

```
Created -> Bound -> Listening (TCP Server)
                 -> Connected (TCP Client)
                 
Created -> Bound (UDP)

任何状态 -> Closed
```

## 为 HTTP 组件准备

这个 Socket API 为构建 HTTP 组件提供了完整的基础：

1. **TCP 连接管理** - HTTP 基于 TCP
2. **数据收发** - HTTP 请求/响应传输
3. **IP 绑定** - 支持多 IP、虚拟主机
4. **连接接受** - HTTP 服务器接受客户端连接

HTTP 组件可以在此基础上实现：
- HTTP 请求/响应解析
- Header 处理
- Body 编解码
- 路由和中间件
- WebSocket 升级

## 测试

新增 5 个测试：
- `test_tcp_socket_creation` - TCP socket 创建
- `test_udp_socket_creation` - UDP socket 创建
- `test_socket_bind` - Socket 绑定
- `test_socket_bind_to_ip` - 绑定到 IP
- `test_socket_lifecycle` - 完整生命周期

总测试数：26 个（之前 21 + 新增 5）

## 示例代码

完整示例文件：
- `examples/socket_with_ip_pool.rs` - Socket 与 IP 池集成的 4 个场景示例

运行示例：
```bash
cargo run --example socket_with_ip_pool
```

## 下一步：HTTP 组件

基于这个 Socket API，可以开发 HTTP 组件，实现：

1. **HTTP Parser** - 解析 HTTP 请求和响应
2. **HTTP Server** - 基于 Socket 的 HTTP 服务器
3. **HTTP Client** - HTTP 客户端
4. **Router** - URL 路由
5. **Middleware** - 中间件系统
6. **WebSocket** - WebSocket 支持

示例架构：
```rust
struct HttpServer {
    socket: Socket,
    router: Router,
}

impl HttpServer {
    fn new(ip: IpAddr, port: u16) -> Self {
        let mut socket = Socket::tcp_v4()?;
        socket.bind_to_ip(ip, port)?;
        socket.listen(1000)?;
        // ...
    }
    
    fn handle_request(&mut self) {
        let mut client = self.socket.accept()?;
        let request_data = client.recv(8192)?;
        let request = HttpRequest::parse(&request_data)?;
        let response = self.router.route(&request)?;
        client.send(&response.to_bytes())?;
    }
}
```

## 兼容性

- ✅ 原生平台（测试环境）
- ✅ WASM32 (wasm32-wasip1)
- ✅ 与现有 WASI socket 实现集成
- ✅ 向后兼容原有 socket API
