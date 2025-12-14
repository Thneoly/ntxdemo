# Host ↔ Guest (scheduler) NIC 数据通路整合方案

> 目标：把当前的 `src/main.rs` 从“只跑 `run-scenario(yaml)` 的组件 runner”，升级成真正的网络 Host Runtime：
>
> - Host 用 `Nic`（`afpacket` / `tpacketv3`）收发二层帧
> - Host 用 `network/stack` 解析 Ethernet→IPv4→UDP
> - Host 把“UDP payload + 元数据”分发给 Wasm Component Guest（scheduler）
> - Guest 返回“要发送的 UDP reply / action”，Host 再做组包（IP/UDP checksum + MAC 封装）并发出
>
> 约束：先保证正确性/可验证闭环；不追求零拷贝（WIT `list<u8>` 仍是 copy），后续再演进 shared-memory。

## 0. 现状盘点（对齐代码）

- Host：`src/main.rs`
  - 支持 Wasmtime Component Model（`Config::wasm_component_model(true)`）
  - 仅加载 `SCHEDULER_COMPONENT`（默认 `plugins/scheduler/wac/scheduler-composed.wasm`）并调用导出 `run-scenario(scenario-yaml: string)`
  - 未做任何 `Nic` I/O

- Userspace NIC：`network/nic/*`
  - `Nic` trait：`send/recv/recv_nonblocking/poll_readable/iface_mac/ifindex/ifname ...`
  - 后端：`AfPacketNic`、`TpacketV3Nic`

- Userspace stack：`network/stack/*`
  - `PacketContext::decode()`：Ethernet→IPv4→UDP
  - `build_udp_reply()`：给定 ingress 的 5-tuple + payload，构造回包帧

- Scheduler WIT：`plugins/scheduler/wit/scheduler/world.wit`
  - world `scheduler-component` 只导出 `run-scenario(...)`
  - 没有“packet ingress/egress”的 ABI

- 参考：`examples/afxdp-wasmtime/afxdp-guest/wit/world.wit`
  - 提供了一个很适合复用的 host→guest 回调模式：`on-packet(meta, data: list<u8>)`

## 1. 最小可行集成（MVP-0）：Host 做 UDP Echo，Guest 只做“payload 变换”

先做一个最小闭环，让架构跑通：

1. Host 从 `Nic` 收到帧 → decode 出 UDP 包。
2. Host 调 `guest.on_udp_datagram(meta, payload)`（新 WIT）。
3. Guest 返回可选 payload（或 action list）。
4. Host 用现有 `build_udp_reply()` 直接用 ingress 的 MAC/IP/PORT 反向组包并发送。

这样可以快速验证：
- WIT 调用链路没问题
- packet decode/encode 没问题
- `tpacketv3` ring + poll 没问题
- veth/netns 闭环可以跑通

### 1.1 新增 WIT：`scheduler:net/packet@0.1.0`

建议把网络 ingress/egress 独立成一个 package，未来也能被别的 guest 复用。

**建议接口（偏 UDP datagram，避免一上来就暴露完整 L2 frame）：**

- `UdpMeta`：携带必要的信息让 guest 做决策/观察
  - l2: `src_mac`, `dst_mac`, `ethertype`
  - l3: `src_ip`, `dst_ip`, `ttl`, `tos`
  - l4: `src_port`, `dst_port`
  - `rx_ifindex`（可选）
  - `timestamp_ns`（可选）

- `on-udp`：host 调用
  - 输入：`meta`, `payload: list<u8>`
  - 输出：`result<option<udp-response>, string>`

- `udp-response`：guest 返回（host 负责封装）
  - `payload: list<u8>`
  - 可选：`override_dst`（极简先不做）

> 为什么不直接返回完整 frame？
> - 目前 `network/stack` 已经有可靠的组包/校验和逻辑；如果让 guest 直接拼 L2 帧，更易出错。
> - 后续如果要做“修改 TTL / NAT / multi-dst”，再逐步扩展 response 结构也不晚。

### 1.2 scheduler-component world 的演进方式（不破坏现有 run-scenario）

保持现有 `run-scenario` 不动，新增一个并行导出接口：

- `export scheduler:net/packet@0.1.0`（带 `on-udp`）

Host 侧优先探测 `on-udp` 是否存在：
- 存在 → 走 NIC path
- 不存在 → 回退到旧的 `run-scenario` runner（兼容测试/演示）

## 2. Host 侧模块拆分建议（src/main.rs → runtime/）

建议把 Host runtime 拆出成清晰分层，避免主函数继续膨胀：

- `src/host/runtime.rs`
  - `HostRuntime::run()` 主循环：poll readable → recv frame → decode → dispatch → maybe send
  - 支持统计（rx/decoded/guest_ok/guest_err/sent/...）

- `src/host/guest.rs`
  - Wasmtime component 装载与函数绑定
  - 提供一个纯 Rust 方法：
    - `fn on_udp(&mut self, meta: UdpMeta, payload: &[u8]) -> Result<Option<Vec<u8>>>`

- `src/host/args.rs`
  - CLI：ifname/backend/udp_port filter/scenario path/component path 等

> 先不做多线程：用单线程 poll + recv + guest call 即可。

## 3. 数据流（端到端）

```text
Nic.recv() -> frame
  -> PacketContext::decode(frame)
     -> DecodedPacket::Udp { src/dst mac/ip/port, payload }
        -> guest.on_udp(meta, payload)
           -> Option<payload>
              -> build_udp_reply(original, new_payload)
                 -> Nic.send(reply_frame)
```

### 3.1 过滤策略

MVP 建议先做 2 个过滤：

- 仅处理 IPv4 + UDP
- 可选：只处理 `dst_port == <listen_port>`（默认 9090，跟现有 echo demo 对齐）

避免把所有 L2 流量都丢给 guest。

## 4. 测试 / 验证计划

### 4.1 本地 veth/netns（复用现有脚本）

- topology：`ntx0` (host) ↔ `ntx1` (in netns `ntxns1`)
- Host 在 root netns 上跑 `ntx`（新 runtime，绑定 `ntx0`）
- Client 在 `ntxns1` 里跑 `traffic-send`（绑定 `ntx1`）

验收标准：
- `traffic-send` RR matched 增长，timeout 为 0 或可解释
- Host 打印 rx/decoded/guest_ok/sent 递增
- tcpdump 能看到 request/reply

#### 4.1.1 最短跑通命令（MVP-0）

> 说明：当前 MVP-0 的 guest `on-udp` 行为是“原样回显 payload”；Host 负责把 tuple 反向并计算 checksum。

Host（root netns，绑定 `ntx0`，监听 UDP dst-port=10001）：

```bash
sudo ./target/debug/Ntx --mode net --iface ntx0 --backend tpacketv3 --port 10001
```

启动后会先在 stderr 打印一行 banner（便于脚本/自动化捕获），类似：

```text
ntx(net) starting: iface=ntx0 backend=TpacketV3 port=10001 snaplen=2048 component=plugins/scheduler/wac/scheduler-composed.wasm
```

Client（netns `ntxns1` 内，绑定 `ntx1`，向 10.0.0.1:10001 发 RR）：

```bash
sudo ./scripts/ntxns1.sh ./target/debug/examples/traffic-send \
  --iface ntx1 \
  --backend tpacketv3 \
  --dst-ips 10.0.0.1 \
  --src-ip 10.0.0.2 \
  --dst-port 10001 \
  --src-port 40000 \
  --rr --arp \
  --pps 50 \
  --count 50
```

### 4.2 单元测试（WIT schema + packet builder）

- 针对 `UdpMeta` 的序列化/边界（IP/port 字节序）
- 针对 Host 侧 decode→meta 映射（给定固定输入帧，meta 字段正确）

## 5. 后续演进（MVP-1/2）

- 零拷贝：
  - 方案 A：host 暴露 shared ring（memory export/import）+ guest 只传 offset/len
  - 方案 B：基于 wasi:io/streams 或自定义 `borrow`（目前 component model 对跨实例共享内存仍需谨慎设计）

- 更丰富的 guest 输出：
  - 多包发送（actions list）
  - 修改 L3/L4 字段（NAT/TTL/DSCP）
  - Host 侧 ARP 缓存/邻居发现（目前 veth/netns 可直接用静态 MAC 或 ARP 学习模块复用）

- 和 scheduler 的 scenario/action 系统融合：
  - `on-udp` 内部把 datagram 转成 event，投递到 eventbus，再由 scheduler 根据 scenario 决策。

---

## 附：建议的 WIT 草案（仅作参考）

> 这段是为了方便后续落地实现，真正提交前请按 scheduler 的包命名习惯整理版本/路径。

```wit
package scheduler:net;

interface packet {
  record ipv4-addr { a: u8, b: u8, c: u8, d: u8 }

  record udp-meta {
    src-mac: list<u8>, // len=6
    dst-mac: list<u8>, // len=6
    src-ip: ipv4-addr,
    dst-ip: ipv4-addr,
    src-port: u16,
    dst-port: u16,
  }

  record udp-response {
    payload: list<u8>,
  }

  on-udp: func(meta: udp-meta, payload: list<u8>) -> result<option<udp-response>, string>;
}

world scheduler-net {
  export packet;
}
```

