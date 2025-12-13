# Userspace UDP echo（AF_PACKET 旁路协议栈示例）

本示例在 **Linux** 上使用 **AF_PACKET (PF_PACKET) raw socket** 在用户态收发二层以太网帧，不走内核 IP 协议栈，并在用户态解析 **Ethernet → IPv4 → UDP**，对指定端口做 echo。

> 适用场景：硬件/驱动不支持 AF_XDP RX/zerocopy 或者你希望先用最通用的 copy 路径把协议栈跑通。

## 背景：从 demo 到网络测试工具

我们最终的定位是一个 **网络测试工具**：

- 能用大量的 **IP / MAC / 端口 / payload 模板** 构造自定义报文
- 支持指定 **源/目的 IP（必要时也支持源/目的 MAC）**
- 支持批量发送（rate/burst/并发），并可选地做收包匹配、统计、RTT

本仓库当前的演进路线是先用兼容性最强的 **AF_PACKET(copy path)** 把“用户态收发与协议解析/构造”打完整闭环，再逐步抽象成可扩展组件（pipeline/handlers），未来再替换更高性能的后端（例如 AF_XDP）时，上层逻辑基本不变。

### 实现方案（当前状态）

当前 `userspace-udp-echo` 已经迁移到 `network::stack` 的可插拔处理模型：

- `PacketContext`：持有 raw frame bytes，并提供 `decode()`
- `Pipeline`：对一次 decode 的结果跑一组 handlers（第一个返回 `Reply` 的 handler 获胜）
- `UdpEchoHandler`：基于过滤条件构造 UDP echo 回包
- `build_udp_reply()`：负责交换 MAC/IP/port 并计算 checksum

对应代码：

- 示例：`examples/userspace-udp-echo.rs`
- pipeline：`network/stack/pipeline.rs`
- handler：`network/stack/udp_echo.rs`

### MVP 待办（网络测试工具）

下面是把 demo 升级为“可配置流量生成/收包统计”的最小可用路径（按优先级）：

- MVP-1（先做这个）：**通用发包器（Traffic Sender）**
  - 能从文件/参数加载大量 `dst_ip`（后续扩展 CIDR/range）
  - 能指定 `src_ip/src_port/dst_port/payload` 构造 UDP 报文并发送
  - 先不做 reply 匹配，重点是“构造 + 发包 + 统计 + 限速”跑通
- MVP-2：**ARP 解析与 dst_mac 自动填充**（或允许用户直接指定 `--dst-mac`）
- MVP-3：**收包 matcher + RTT/丢包统计**（token 关联请求/响应）
- MVP-4：**场景化/数据驱动**（yaml/json 场景文件；支持 zip/product 组合、rate/burst、并发 worker）

## Traffic Sender（MVP-1/2）

仓库中另一个配套示例是 `traffic-send`：它用于“按数据集构造 UDP 报文并发送”，适合做压测/扫描/探测的基础组件。

### 目标 IP 来源：`--dst-ips`（推荐）与 `--dst-ip-file`

`traffic-send` 现在支持两种方式提供目标 IP：

- **`--dst-ips SPEC`（推荐）**：直接在命令行提供目标集合。`SPEC` 支持：
  - 单个 IP：`10.0.0.1`
  - CIDR：`10.0.0.0/24`
  - Range（包含端点）：`10.0.0.10-10.0.0.20`
  - 该参数可重复传入，也支持逗号分隔：`--dst-ips 10.0.0.1,10.0.0.2`
- **`--dst-ip-file FILE`**：从文件加载（每行一个 IP，支持 `#` 注释）。

优先级（从高到低）：

1. CLI `--dst-ips`
2. scenario `dst_ips`
3. CLI `--dst-ip-file`
4. scenario `dst_ip_file`

### MVP-2：启用 ARP 自动解析 dst_mac

在二层网络里，如果你只有目标 IP（`dst_ip`），通常需要先通过 ARP 学到目标机器的 MAC 才能稳定发到对端。

`traffic-send` 支持两种方式：

- **显式指定 `--dst-mac`**（最直接，常用于可控环境/旁路测试）
- **启用 `--arp`**（发送 ARP request 并缓存 reply；无缓存时会同步等待一小段时间）

示例（启用 ARP + CIDR 目标）：

```bash
sudo -E cargo run --example traffic-send -- \
  --iface eno1 \
  --dst-ips 192.168.1.0/24 \
  --arp \
  --arp-timeout-ms 800 \
  --arp-ttl-s 60 \
  --dst-port 10001 \
  --pps 1000 \
  --count 10000
```

示例（手工指定 dst MAC，绕过 ARP；目标仍可用 `--dst-ips` 或 `--dst-ip-file`）：

```bash
sudo -E cargo run --example traffic-send -- \
  --iface eno1 \
  --dst-ips 192.168.1.10-192.168.1.20 \
  --dst-mac aa:bb:cc:dd:ee:ff \
  --dst-port 10001
```

## MVP-3 预告：收包 matcher + RTT/丢包统计

MVP-3 的目标是把 `traffic-send` 从“只管发”升级为“可测量”的网络测试工具：

- 在 UDP payload 中注入 token（例如递增的 u64 + magic）
- 维护 outstanding table（token → 发送时间/目标）
- 在接收路径解析 token 并关联，输出 RTT / 丢包 / p95 等统计

> 注意：如果你伪造了源 IP（src_ip 不是本机可达地址），正常网络下回包可能不会回到你，这类场景可以用 fire-and-forget 模式或旁路抓包方式验证。

## MVP-4：场景化（Scenario 文件）

当你需要用“很多 IP/端口/模式组合”重复跑测试时，推荐用 `--scenario` 让 `traffic-send` 从 YAML 文件加载配置。

规则：

- Scenario 提供默认值
- CLI 显式传入的参数会覆盖 scenario（方便临时改端口/pps）

示例 `scenario.yaml`：

```yaml
version: 1
iface: eno1

# 二选一：dst_ips 或 dst_ip_file（若同时提供，dst_ips 优先）
dst_ips:
  - "192.168.1.0/24"

# dst_ip_file: dst_ips.txt

src_ip: 192.168.1.10
src_port: 40000
dst_port: 10001

payload: "ntx-traffic"
pps: 1000
count: 10000

arp:
  enabled: true
  timeout_ms: 800
  ttl_s: 60

rr:
  enabled: true
  timeout_ms: 500
  poll_budget: 256
```

运行：

```bash
sudo -E cargo run --example traffic-send -- --scenario scenario.yaml
```

CLI 覆盖示例（临时改 targets）：

```bash
sudo -E cargo run --example traffic-send -- --scenario scenario.yaml --dst-ips 10.0.0.0/24
```

## 目标与边界

- **不占用/不配置接口 IP**，不与内核协议栈“抢”网卡。
- 用户态只处理符合过滤条件的流量：
  - `dst_mac == 本机网卡 MAC` **或** `dst_mac == ff:ff:ff:ff:ff:ff`（broadcast）
  - IPv4
  - UDP
  - `udp.dst_port == --port`（默认 10001）
- 回包时会交换 src/dst 的 MAC、IP、端口，并计算 IPv4 header checksum 与 UDP checksum。

## 运行

需要 root 权限创建 raw socket：

```bash
sudo -E cargo run --example userspace-udp-echo -- --iface eno1 --port 10001
```

如果你的环境里 `sudo -E` 会被忽略（例如 sudo-rs），建议直接使用二进制 + setcap（见下方 `USAGE.md`），或者用 cargo 的绝对路径执行。

参数：

- `--iface`：要绑定的网卡（默认 `eno1`）
- `--port`：要 echo 的 UDP 端口（默认 `10001`）
- `--snaplen`：每次 `recv()` 读取的最大帧长度（默认 `2048`）
- `--verbose`：命中 echo 时打印一行日志（会有少量输出，便于调试）

启动后会打印接口的 `ifindex` 与 MAC 地址。

## 如何验证

### 推荐方式：两台机器同一二层网络

1) **目标机（运行示例的机器）** 启动 echo：

```bash
sudo -E cargo run --example userspace-udp-echo -- --iface eno1 --port 10001
```

2) **另一台机器** 发送 UDP 到目标机 IP/端口：

```bash
echo -n 'ping' | nc -u -w1 <TARGET_IP> 10001
```

3) 观察：

- 发送端若能收到回包，说明 echo 成功。
- 如果你想观察二层/三层细节，建议在目标机额外开一个抓包窗口：

```bash
sudo tcpdump -ni eno1 -vv udp port 10001
```

### 单机验证（可选）

如果同机发送 UDP，回包可能仍然能看到，但取决于路由/ARP/内核发送路径是否会从物理网卡出去；旁路 raw socket 的 demo 更推荐跨主机验证。

## 常见问题排查

- **退出/报错：权限不足**：raw socket 需要 root，必须 `sudo`。
- **收不到包**：
  - `--iface` 是否选对（例如 `ip link` 看实际接口名）；
  - 目标端口是否一致；
  - 先用 `tcpdump -ni <iface>` 确认网卡上确实有流量。
- **对端收不到回包**：
  - 检查交换机/VLAN/二层连通性；
  - 用 `tcpdump -vv -XX` 看回包是否正确（MAC / IP / port / checksum）；
  - 某些网卡 offload/对端协议栈可能会对异常帧更严格。

## 相关代码

- 示例入口：`examples/userspace-udp-echo.rs`
- NIC 后端：`network/nic/afpacket.rs`
- 协议层：
  - Ethernet：`network/mac/ethernet.rs`
  - IPv4：`network/ip/ipv4.rs`
  - UDP：`network/udp/udp.rs`

## 单测

- `tests/network_checksums.rs`：checksum 与 parse/write 的 roundtrip + synthetic pipeline。
- `tests/arp.rs`：ARP request/reply 的 build/parse 以及 ARP cache TTL 过期行为。

运行单测：

```bash
cargo test -q
```
