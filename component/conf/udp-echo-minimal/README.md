### udp-echo-minimal（scheduler 最小配置）

这个目录用于给 `component/scheduler` 的入口 `run(config-dir)` 提供最小配置：
- **入口文件**：`scenario.yaml`
- **目标**：跑通 `action -> wait(packet.rx) -> end -> user.exit` 事件闭环

#### 你需要改的地方
- `workbook.resources[udp-target].properties.peer_ip/peer_port`
- `peer_mac`：
  - 如果 host 侧 ARP cache 已经有 `peer_ip -> peer_mac`，可以把 `peer_mac` 删除（scheduler 会 best-effort 调用 `resources.resolve-peer-mac`）
  - 否则请填真实 `peer_mac`

#### 如何运行
把此目录的路径作为 `config-dir` 传给 scheduler 组件的导出函数：
- `scheduler-component.run(config-dir: string) -> result<_, string>`


