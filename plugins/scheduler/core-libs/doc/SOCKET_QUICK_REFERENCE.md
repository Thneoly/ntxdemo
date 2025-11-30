# Socket API 快速参考

## 基本用法

### TCP 服务器

```rust
use scheduler_core::{Socket, IpPool, ResourceType};

// 1. 分配 IP
let mut pool = IpPool::new("pool");
pool.add_cidr_range("192.168.1.0/24")?;
let ip = pool.allocate("tenant", "server", ResourceType::Vm("vm1".into()))?;

// 2. 创建并绑定 socket
let mut server = Socket::tcp_v4()?;
server.bind_to_ip(ip, 8080)?;
server.listen(100)?;

// 3. 接受连接
let mut client = server.accept()?;

// 4. 收发数据
let data = client.recv(1024)?;
client.send(b"Response")?;

// 5. 关闭
client.close()?;
server.close()?;
```

### TCP 客户端

```rust
// 1. 创建 socket
let mut client = Socket::tcp_v4()?;

// 2. （可选）绑定源 IP
let ip = pool.allocate("tenant", "client", ResourceType::Container("c1".into()))?;
client.bind_to_ip(ip, 0)?; // 端口 0 = 自动选择

// 3. 连接
let addr = SocketAddress::new("192.168.1.1", 8080);
client.connect(addr)?;

// 4. 收发
client.send(b"Request")?;
let response = client.recv(4096)?;

// 5. 关闭
client.close()?;
```

### UDP Socket

```rust
// 1. 创建并绑定
let mut udp = Socket::udp_v4()?;
udp.bind_to_ip(ip, 9000)?;

// 2. 发送到地址
let target = SocketAddress::new("192.168.1.10", 9001);
udp.send_to(b"Data", target)?;

// 3. 从任意地址接收
let (data, sender) = udp.recv_from(1024)?;

// 4. 关闭
udp.close()?;
```

## API 速查表

### 创建 Socket

| 方法 | 说明 |
|------|------|
| `Socket::tcp_v4()` | TCP IPv4 |
| `Socket::tcp_v6()` | TCP IPv6 |
| `Socket::udp_v4()` | UDP IPv4 |
| `Socket::udp_v6()` | UDP IPv6 |

### 绑定 IP

| 方法 | 说明 |
|------|------|
| `bind_to_ip(ip, port)` | 绑定 IP 和端口 |
| `bind(SocketAddress)` | 绑定地址 |
| `bind_with_binding(binding, port)` | 使用 IpBinding |

### TCP 操作

| 方法 | 说明 | 适用 |
|------|------|------|
| `listen(backlog)` | 监听连接 | TCP 服务器 |
| `connect(addr)` | 连接服务器 | TCP 客户端 |
| `accept()` | 接受连接 | TCP 服务器 |

### 数据传输

| 方法 | 说明 | 适用 |
|------|------|------|
| `send(data)` | 发送数据 | TCP/UDP |
| `recv(max_len)` | 接收数据 | TCP/UDP |
| `send_to(data, addr)` | 发送到地址 | UDP |
| `recv_from(max_len)` | 从地址接收 | UDP |

### 查询信息

| 方法 | 返回 |
|------|------|
| `local_ip()` | `Option<IpAddr>` |
| `local_port()` | `Option<u16>` |
| `remote_addr()` | `Option<&SocketAddress>` |
| `protocol()` | `SocketProtocol` |
| `family()` | `AddressFamily` |
| `is_connected()` | `bool` |
| `is_bound()` | `bool` |
| `is_listening()` | `bool` |

## 常见模式

### HTTP 服务器基础

```rust
// 分配 IP
let http_ip = pool.allocate("http", "server", 
    ResourceType::Custom("http".into()))?;

// 创建服务器
let mut server = Socket::tcp_v4()?;
server.bind_to_ip(http_ip, 80)?;
server.listen(1000)?;

loop {
    let mut client = server.accept()?;
    
    // 接收 HTTP 请求
    let request = client.recv(8192)?;
    
    // 处理并响应
    let response = b"HTTP/1.1 200 OK\r\n\r\nHello";
    client.send(response)?;
    
    client.close()?;
}
```

### 客户端带重试

```rust
fn connect_with_retry(addr: SocketAddress, retries: u32) 
    -> Result<Socket, SocketError> 
{
    for i in 0..retries {
        let mut sock = Socket::tcp_v4()?;
        match sock.connect(addr.clone()) {
            Ok(_) => return Ok(sock),
            Err(e) if i < retries - 1 => {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(SocketError::ConnectionRefused)
}
```

### UDP 广播模式

```rust
// 接收端
let mut receiver = Socket::udp_v4()?;
receiver.bind_to_ip("0.0.0.0".parse()?, 9000)?;

// 发送端
let mut sender = Socket::udp_v4()?;
let broadcast = SocketAddress::new("255.255.255.255", 9000);
sender.send_to(b"Broadcast message", broadcast)?;
```

## 与 IP 池集成模式

### 模式 1: 临时分配

```rust
let ip = pool.allocate("temp", "socket1", 
    ResourceType::Custom("socket".into()))?;
let mut sock = Socket::tcp_v4()?;
sock.bind_to_ip(ip, 8080)?;
// ... 使用 socket
sock.close()?;
pool.release_by_subid("temp", "socket1")?;
```

### 模式 2: 长期持有

```rust
struct Service {
    ip: IpAddr,
    socket: Socket,
}

impl Service {
    fn new(pool: &mut IpPool) -> Result<Self, Box<dyn Error>> {
        let ip = pool.allocate("service", "main", 
            ResourceType::Custom("service".into()))?;
        let mut socket = Socket::tcp_v4()?;
        socket.bind_to_ip(ip, 8080)?;
        Ok(Self { ip, socket })
    }
}
```

### 模式 3: 多 Socket 共享 IP

```rust
// 同一个 IP，不同端口
let ip = pool.allocate("app", "multi", 
    ResourceType::Vm("app-vm".into()))?;

let mut http = Socket::tcp_v4()?;
http.bind_to_ip(ip, 80)?;

let mut https = Socket::tcp_v4()?;
https.bind_to_ip(ip, 443)?;

let mut admin = Socket::tcp_v4()?;
admin.bind_to_ip(ip, 8080)?;
```

## 错误处理

```rust
match sock.connect(addr) {
    Ok(_) => println!("Connected"),
    Err(SocketError::ConnectionRefused) => {
        eprintln!("Connection refused");
    }
    Err(SocketError::Timeout) => {
        eprintln!("Connection timeout");
    }
    Err(SocketError::NetworkUnreachable) => {
        eprintln!("Network unreachable");
    }
    Err(e) => {
        eprintln!("Error: {}", e);
    }
}
```

## 最佳实践

1. ✅ **总是关闭 socket** - 使用 `close()` 或依赖 Drop trait
2. ✅ **检查状态** - 使用 `is_bound()`, `is_connected()` 等
3. ✅ **错误处理** - 妥善处理网络错误
4. ✅ **IP 池管理** - 使用后释放 IP
5. ✅ **超时设置** - 为长连接设置超时（未来功能）
6. ✅ **资源复用** - 考虑连接池模式

## 状态转换

```
[Created]
   |
   | bind_to_ip()
   v
[Bound]
   |
   +---> listen() ---> [Listening] ---> accept() ---> [Connected]
   |
   +---> connect() ---> [Connected]
   
任何状态 ---> close() ---> [Closed]
```

## 下一步

1. 查看完整文档：`doc/SOCKET_IP_INTEGRATION.md`
2. 运行示例：`cargo run --example socket_with_ip_pool`
3. 查看 HTTP 组件设计（即将推出）
