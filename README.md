# Ntx (演示)

这是一个演示仓库，包含若干插件（位于 `plugins/`）和一个顶层的 Rust 可执行/库。仓库中的 build 脚本会监视插件目录的变动，并在必要时为指定插件触发子构建或运行脚本（例如将插件构建为 wasm 目标）。

## 主要特性
- 自动检测 `plugins/core`、`plugins/demo`、`plugins/tcp-client`、`plugins/wac` 的文件变更。
- 对 `core`、`demo`、`tcp-client` 在变更时执行 `cargo build --target wasm32-wasip2`。
- 对 `wac` 在变更时执行 `run.sh`（通过 `sh run.sh` 运行）。
- 状态文件写入到 Cargo 的 `OUT_DIR`（或回退到 `target/`），避免将构建状态提交到版本控制。
- 在 CI 或快速本地构建时可以通过环境变量关闭插件自动构建（参见下文）。

## 环境准备
安装一些常用工具：

```bash
cargo install cargo-binstall
cargo binstall cargo-component wit-bindgen-cli wasmtime-cli wasm-tools wit-deps-cli cargo-expand -y
```

（按需安装上面工具中的子集。）

## 构建与运行

在仓库根目录运行：

```bash
cargo build
```

或者运行：

```bash
# 需要启动 TCP 服务并监听 8080 端口, 可以使用 nc 工具
cargo run
```

这些命令会触发顶层构建并执行 `build.rs`。当 `plugins/*` 下有变更时，`build.rs` 可能会在对应子目录执行 `cargo build --target wasm32-wasip2` 或 `sh run.sh`。

### Scheduler 场景运行

仓库内置了一个 scheduler 组件（`plugins/scheduler`）。你可以通过两种方式运行它：

1. **主程序（自动绑定）**

	```bash
	# 使用默认场景 plugins/scheduler/res/simple_scenario.yaml
	cargo run

	# 或指定自定义场景 YAML
	cargo run -- plugins/scheduler/res/http_scenario.yaml
	```

	- 默认会加载 `plugins/scheduler/target/wasm32-wasip2/debug/scheduler_composed.wasm`。
	- 如需替换组件，可设置 `SCHEDULER_COMPONENT` 环境变量：

	  ```bash
	  SCHEDULER_COMPONENT=path/to/your_scheduler.wasm cargo run -- path/to/scenario.yaml
	  ```

2. **手动调用模式（`src/bin/call.rs`）**

	该二进制展示了如何不用 bindgen 手动查找并调用组件导出的 `run-scenario` 函数：

	```bash
	cargo run --bin call -- plugins/scheduler/res/http_scenario.yaml
	```

	行为与主程序一致，同样支持 `SCHEDULER_COMPONENT` 环境变量来指向不同的 `.wasm` 组件。输出会显示 YAML 文本长度与组件返回的 Summary/错误信息。

### Runner 组件
- 详细运行手册：`plugins/runner/docs/plan/runbook.md`
- 数据模型说明：`plugins/runner/docs/plan/sparse_model.md`
- 特性开关策略：`plugins/runner/docs/plan/feature_flags.md`
- 快速构建：
	```bash
	cargo build -p runner --target wasm32-wasip2
	```
- 快速运行：
	```bash
	cd plugins/runner
	RUNTIME_SCENARIO=http_tri_phase_demo ./run.sh
	```

## 控制插件构建（跳过/启用）

如果你想在 CI 或快速本地开发时跳过插件自动构建（仅构建顶层），可以设置环境变量 `DISABLE_PLUGIN_BUILDS`：

```bash
# 跳过插件自动构建（值为 1 或 true 将被视为启用）
DISABLE_PLUGIN_BUILDS=1 cargo build
```

当 `DISABLE_PLUGIN_BUILDS` 被设置时，build 脚本仍会计算并记录插件目录的哈希（状态会更新），但不会执行 `cargo build` 或 `run.sh`。

如果你更希望在跳过时不更新状态（以便重新启用时仍然触发一次完整构建），请告诉我，我可以修改行为。

## 状态文件位置

插件变更的状态记录（哈希）会写到 Cargo 提供的 `OUT_DIR` 目录下的 `plugin_build_state` 文件。如果 `OUT_DIR` 不可用，脚本会回退到仓库的 `target/` 目录。该文件默认不会被添加到 VCS，但如果你希望将其放在别处（或加入 `.gitignore`），可以调整脚本配置。

## 插件目录与忽略规则

为避免扫描大量构建产物，脚本会忽略以下目录：

- `*.git`、`node_modules`（任意位置），
- 严格忽略 `target` 目录（尤其是 `plugins/*/target` 下的子目录）。

## 故障排查

- 如果子插件构建失败，当前实现会让顶层构建失败（panic），并在终端输出子进程的退出状态与错误信息。
- 若要将子构建失败改为仅打印警告并继续主构建，请说明，我可以把错误处理改为非致命日志。
- 若发现 build 脚本没有触发预期子构建，确认：
	- 你是否修改了 `plugins/<name>` 下的文件（注意 `target` 被忽略）；
	- 是否设置了 `DISABLE_PLUGIN_BUILDS`；
	- 检查 `OUT_DIR/plugin_build_state` 中保存的哈希以判断上次构建状态。

## 示例 — 快速开发流程

1. 修改插件 `plugins/core/src/lib.rs`。
2. 在仓库根运行（默认会触发插件子构建）：

```bash
cargo build
```

3. 若要快速跳过插件构建：

```bash
DISABLE_PLUGIN_BUILDS=1 cargo build
```

## 其他

可以使用 `nc -l 127.0.0.1 8080` 监听端口 开启TCP 监听端口。

## Userspace UDP echo（AF_PACKET 旁路协议栈示例）

这个示例展示“**不走内核 IP 协议栈**、只使用 Linux 的 AF_PACKET raw socket 在用户态收发二层帧”，并在用户态解析 **Ethernet → IPv4 → UDP**，对指定端口做 echo。

特点/约束：

- **不会配置/占用接口 IP**，也不会把接口从内核接管走（Plan B：旁路/观测 + 自己回包）。
- 需要 root 权限创建 raw socket（`sudo`）。
- 仅对 `dst_mac == 本机网卡 MAC` 或 `dst_mac == broadcast` 的帧响应；并且仅对 `udp.dst_port == 指定端口` 回包。

### 运行

```bash
sudo -E cargo run --example userspace-udp-echo -- --iface eno1 --backend afpacket --port 10001
```

参数：

- `--iface`：要绑定的网卡（默认 `eno1`）
- `--port`：要 echo 的 UDP 端口（默认 `10001`）
- `--snaplen`：每次 `recv()` 读取的最大帧长度（默认 `2048`）

启动后会打印接口的 `ifindex` 与 MAC 地址。

### 如何验证（推荐：两台机器同一二层网络）

因为这是“二层收包 + 自己构造回包”，所以最稳定的验证方式是：

1) **在目标机**（运行示例的机器）启动 echo：

```bash
sudo -E cargo run --example userspace-udp-echo -- --iface eno1 --backend afpacket --port 10001
```

2) **在另一台机器**（同一交换机/同一二层网络）向目标机发送 UDP：

```bash
echo -n 'ping' | nc -u -w1 <TARGET_IP> 10001
```

你应该能在发送端看到回包，或在抓包里看到 reply（可用 `tcpdump -ni eno1 udp port 10001` 辅助观察）。

### 常见问题排查

- **运行直接失败 / 权限不足**：AF_PACKET raw socket 需要 root，确保用了 `sudo`。
- **收不到包**：确认 `--iface` 选对了网卡；网卡在正确的 VLAN/二层网络；也可以用 `tcpdump -ni <iface>` 看是否有流量。
- **发出后对端收不到回包**：可能是交换机/网卡 offload、或对端 OS 丢弃了校验和异常的包。
	- 本示例会计算 IPv4 header checksum 和 UDP checksum。
	- 如仍异常，先用 `tcpdump -vv -XX` 观察回包字段是否正确。


```shell
基于 AF_XDP + ring shared memory + WASM (wasip2) 实现： “零拷贝架构”应该长这样：
	NIC → XDP → AF_XDP → (shared UMEM page) → Rust Host → (memoryview import) → WASM Guest


数据通路特点：

	NIC DMA → 用户态（零拷贝）

	Rust Host 和 WASM 可以共用同一块内存（Wasm GC & shared memory）

	不需要来回 memcpy

可以实现 Mac → IP → TCP → HTTP 全部自己写; 
WASM 插件架构：

	Host（Rust）控制 IO & memory

	Guest（WASM）处理协议 & 业务

	AF_XDP 保证最小延迟和最高吞吐 请给出示例  包含 XDP/eBPF 程序（把包重定向到 AF_XDP）、Rust Host 用户态（创建 UMEM + AF_XDP sockets，并从 ring 读取包）以及 build / run 指引

https://github.com/aya-rs/aya
https://aya-rs.dev/book/start/hello-xdp 这是一个教程
```