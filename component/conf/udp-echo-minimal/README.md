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

#### Actions Catalog（给前端的动作清单/表单 schema）

本目录包含一个固定示例 `actions-catalog.json`，用于前端/平台在**不执行 wasm** 的情况下获取 action 列表与参数 schema。

如需用当前代码重新生成（推荐）：

```bash
./component/conf/udp-echo-minimal/gen-actions-catalog.sh
```

它会：
1) 构建 `actions-executor` 的 WASIp2 component
2) 调用其自描述接口 `schema-version/list-actions/describe-action`
3) 写入 `component/conf/udp-echo-minimal/actions-catalog.json`


