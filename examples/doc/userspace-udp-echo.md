# Userspace UDP echo（AF_PACKET 旁路协议栈示例）

本示例在 **Linux** 上使用 **AF_PACKET (PF_PACKET) raw socket** 在用户态收发二层以太网帧，不走内核 IP 协议栈，并在用户态解析 **Ethernet → IPv4 → UDP**，对指定端口做 echo。

> 适用场景：硬件/驱动不支持 AF_XDP RX/zerocopy 或者你希望先用最通用的 copy 路径把协议栈跑通。

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
