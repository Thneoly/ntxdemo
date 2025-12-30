
# actions-catalog-gen

这个工具用于**在宿主侧**实例化 `actions-executor` 的 **WASIp2 component**，并调用它的自描述接口生成 **Actions Catalog JSON**：

- `schema-version()`
- `list-actions()`
- `describe-action(action-id)`

这条路径是我们选定的 “**不维护 manifest**” 方案：

- **可执行 component** 本身就是 action 列表与 schema 的唯一真相源
- 平台/Host 负责把 catalog 转成前端可直接消费的 JSON（并做缓存）

## 依赖

- Rust toolchain for this repo
- The `wasm32-wasip2` target installed

## 构建 actions-executor 组件

From the repo root:

```bash
cargo build -p actions-executor --target wasm32-wasip2
```

The output component is expected at:

- `target/wasm32-wasip2/debug/actions_executor.wasm`

(If you build `--release`, adjust the path accordingly.)

## 构建并运行生成器

### 输出到 stdout

```bash
cargo run -p actions-catalog-gen -- target/wasm32-wasip2/debug/actions_executor.wasm
```

### 写入文件

```bash
cargo run -p actions-catalog-gen -- \
	target/wasm32-wasip2/debug/actions_executor.wasm \
	component/conf/udp-echo-minimal/actions-catalog.json
```

第二个参数可选：不传时输出到 stdout。

推荐把输出落在场景目录下，便于示例/联调：

- `component/conf/udp-echo-minimal/actions-catalog.json`

（后续也可以改成平台固定目录，并按 **component hash + schema-version** 做缓存。）

## 输出格式

The tool emits a stable JSON object of the form:

```json
{
	"schema-version": 1,
	"executor": {
		"component-path": "..."
	},
	"actions": [
		{
			"summary": {
				"id": "udp-send-reply",
				"title": "...",
				"description": "..."
			},
			"spec": {
				"id": "udp-send-reply",
				"title": "...",
				"description": "...",
				"params-schema-json": "{...JSON Schema as string...}",
				"default-params-json": "{...defaults as string...}",
				"capabilities": [
					{ "debug": "ActionCapability::..." }
				]
			}
		}
	]
}
```

说明：

- `params-schema-json` 和 `default-params-json` 是 **JSON 字符串**（前端可直接 `JSON.parse`）。
- `capabilities` 目前是 debug 字符串列表（`ActionCapability::...`），目的是避免这个工具强依赖 WIT 生成代码的字段命名。

后续如果前端需要把 capabilities 当“可筛选标签”使用，建议升级为稳定的字符串枚举（例如 `emits-packet-tx-request` 这种 kebab-case）。

## 常见问题

### 找不到组件文件 / 路径不对

Make sure you built the component with the same profile/target you’re referencing.

- debug build: `target/wasm32-wasip2/debug/actions_executor.wasm`
- release build: `target/wasm32-wasip2/release/actions_executor.wasm`

### WASI / import linking 错误

这个工具内置了一个最小的 `event-bus` import 的 no-op 实现，让 component 能成功实例化。

如果后续 actions-executor 的 world 新增/变更 imports，这个工具需要同步补齐对应的 host stub。

## 下一步建议（推荐顺序）

1. **把生成的 catalog 纳入 udp-echo-minimal 的联调流程**
	- 约定 `component/conf/udp-echo-minimal/actions-catalog.json` 为示例目录的默认落盘结果
	- 前端/平台读取这个 JSON，生成 palette + 表单

2. **加一个轻量回归检查**
	- CI 或测试中跑一次 generator，断言至少包含 `udp-send-reply` / `udp-schedule-send`，防止 catalog API 回退/破坏

3. **稳定 capabilities 输出**
	- 把 debug string 改成稳定的字符串枚举，避免前端依赖 Rust Debug 输出

