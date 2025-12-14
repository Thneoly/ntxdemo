# Echo 场景快速启动指南

## 概述

本指南展示如何快速启动 Echo Server 和 Echo Client 的最简形式实现。

## 前置条件

- Rust 工具链已安装
- `wasm32-wasip2` 目标已安装：`rustup target add wasm32-wasip2`
- WAC 工具已安装：`cargo install wac-cli`
- 网络权限（需要 root）

## 快速开始

### 第 1 步：编译所有组件

```bash
cd /home/cc/Desktop/code/GitHub/Ntx
chmod +x plugins/scheduler/scripts/build-echo.sh
./plugins/scheduler/scripts/build-echo.sh
```

**预期输出**：
```
╔════════════════════════════════════════════════════╗
║        Echo 场景 WAC 编排编译脚本                  ║
╚════════════════════════════════════════════════════╝
[1/6] Building core-libs...
[2/6] Building eventbus...
[3/6] Building actions-executor-server...
[4/6] Building actions-executor-client...
[5/6] Building scheduler...
[6/6] WAC orchestration...

╔════════════════════════════════════════════════════╗
║                 编译完成！                         ║
╚════════════════════════════════════════════════════╝

[+] Output files:
    • .../plugins/scheduler/wac/echo-server.wasm
    • .../plugins/scheduler/wac/echo-client.wasm
```

### 第 2 步：建立虚拟网络拓扑

```bash
# 创建 veth 对（如果脚本存在）
cd /home/cc/Desktop/code/GitHub/Ntx
if [ -f scripts/ntx-veth-up.sh ]; then
    sudo ./scripts/ntx-veth-up.sh
fi

# 或手动创建
sudo ip link add veth1 type veth peer name veth2
sudo ip netns add ns1
sudo ip netns add ns2
sudo ip link set veth1 netns ns1
sudo ip link set veth2 netns ns2
sudo ip netns exec ns1 ip addr add 10.0.0.1/24 dev veth1
sudo ip netns exec ns2 ip addr add 10.0.0.2/24 dev veth2
sudo ip netns exec ns1 ip link set veth1 up
sudo ip netns exec ns2 ip link set veth2 up
```

### 第 3 步：启动 Echo Server（主机 1）

**终端 1**：
```bash
cd /home/cc/Desktop/code/GitHub/Ntx

# 使用 echo-server 模式
sudo ./target/debug/Ntx \
  --mode server \
  --iface veth1 \
  --port 10001 \
  --component ./plugins/scheduler/wac/echo-server.wasm

# 或在网络命名空间中运行（如果使用了脚本）
# sudo ip netns exec ns1 ./target/debug/Ntx \
#   --mode server \
#   --iface veth1 \
#   --port 10001 \
#   --component ./plugins/scheduler/wac/echo-server.wasm
```

**预期输出**：
```
ntx(echo-server) starting: iface=veth1 port=10001 component=./plugins/scheduler/wac/echo-server.wasm
ntx(echo-server): iface=veth1 mac=aa:bb:cc:dd:ee:ff port=10001 backend=AfPacket
[echo-server] rx=0 udp=0 processed=0 sent=0
...
[echo-server] rx=100 udp=98 processed=98 sent=98
```

### 第 4 步：启动 Echo Client（主机 2）

**终端 2**：
```bash
cd /home/cc/Desktop/code/GitHub/Ntx

# 使用 echo-client 模式
sudo ./target/debug/Ntx \
  --mode client \
  --iface veth2 \
  --server-ip 10.0.0.1 \
  --server-port 10001 \
  --count 100 \
  --pps 50 \
  --component ./plugins/scheduler/wac/echo-client.wasm

# 或在网络命名空间中运行
# sudo ip netns exec ns2 ./target/debug/Ntx \
#   --mode client \
#   --iface veth2 \
#   --server-ip 10.0.0.1 \
#   --server-port 10001 \
#   --count 100 \
#   --pps 50 \
#   --component ./plugins/scheduler/wac/echo-client.wasm
```

**预期输出**：
```
ntx(echo-client) starting: iface=veth2 server=10.0.0.1:10001 count=100 pps=50
[echo-client] TODO: Implement full client mode with WASM integration
```

## 验证流程

### Server 端验证

检查 Server 是否接收和转发包：

```bash
# 在 Server 终端查看统计
# 应该看到 rx 和 sent 计数增加
```

### Client 端验证

检查 Client 是否发送请求：

```bash
# 监听网络
sudo tcpdump -i veth2 -nn "port 10001" -v

# 预期看到 UDP 包往来
# 10.0.0.2.xxxxx > 10.0.0.1.10001: UDP, length ...
# 10.0.0.1.10001 > 10.0.0.2.xxxxx: UDP, length ...
```

## 故障排除

### 问题 1：Permission denied

```
Error: failed to open NIC: Permission denied
```

**解决方案**：使用 `sudo` 运行

### 问题 2：wac 工具未找到

```
