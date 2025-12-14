# Phase2 WASM Echo 联调记录

## 概览
- Echo Server/Client 现已走 WASM 组件：server 导出 `server/on-packet-received`，client 导出 `client/generate`。
- 客户端已改为构造完整 Ethernet+IPv4+UDP 帧（默认源 IP 10.0.0.2，目标 IP 为 `--server-ip`，目标 MAC 使用广播以避免 ARP 依赖）。
- 在 ntx0/ntx1 拓扑下，端到端验证：`sent=5 matched=5`，WASM 导出成功调用。

## WIT 定义摘要
- `plugins/scheduler/actions-executor-server/wit/actions-executor-server.wit`
```wit
package scheduler:actions-executor;

interface server {
    on-packet-received: func(payload: list<u8>) -> result<list<u8>, string>;
}

world actions-executor-server {
    export server;
}
```
- `plugins/scheduler/actions-executor-client/wit/actions-executor-client.wit`
```wit
package scheduler:actions-executor;

interface client {
    generate: func(count: u32, pps: u32) -> result<u32, string>;
}

world actions-executor-client {
    export client;
}
```

## 构建 WASM 组件
在各自目录构建 wasm32-wasip2 release：
```bash
cd plugins/scheduler/actions-executor-server
cargo build --target wasm32-wasip2 --release

cd ../actions-executor-client
cargo build --target wasm32-wasip2 --release
```

## 部署产物
将生成的组件复制到运行路径：
```bash
cp plugins/scheduler/target/wasm32-wasip2/release/scheduler_actions_executor_server.wasm \
   plugins/scheduler/wac/echo-server.wasm
cp plugins/scheduler/target/wasm32-wasip2/release/scheduler_actions_executor_client.wasm \
   plugins/scheduler/wac/echo-client.wasm
```

## 运行与验证（ntx0/ntx1 拓扑）
> 需先运行 `scripts/ntx-veth-up.sh`，确保 host:ntx0=10.0.0.1，netns:ntx1=10.0.0.2。

```bash
# 服务端（host）
sudo ./target/debug/Ntx --mode server --iface ntx0 --backend afpacket --port 10001

# 客户端（netns: ntxns1）
NTX_CLIENT_IP=10.0.0.2 \
sudo ip netns exec ntxns1 ./target/debug/Ntx --mode client --iface ntx1 --backend afpacket \
  --server-ip 10.0.0.1 --server-port 10001 --count 5 --pps 5
```

预期日志：
- Server: `Echo server export resolved: server/on-packet-received`，UDP 计数递增。
- Client: `WASM generate() returned count=5`，`Received matching seq=...`，`[result] sent=5 matched=5 ...`。

## 常见问题排查
- **导出缺失/链接失败**：确保 guest 侧使用 `wit_bindgen::generate!` 并 `export!(Server/Client)`；`wasm-tools component wit <wasm>` 可查看导出名。
- **instantiate 失败**：多因 imports 不匹配或组件未导出 world；重新检查 WIT 和导出宏。
- **UDP=0 / 收不到包**：旧版客户端发送裸 payload；请使用当前版本（已封装 Ethernet+IPv4+UDP）。如需自定义源 IP，可设置 `NTX_CLIENT_IP`。
- **MAC 解析**：客户端默认广播目的 MAC 以避免 ARP，若环境要求指定目的 MAC，可在代码中按需扩展解析。

## 后续可选优化
- 在 WASM 客户端生成 payload/校验逻辑，减少 host 回退路径。
- 完善错误码与统计上报，便于自动化回归。
