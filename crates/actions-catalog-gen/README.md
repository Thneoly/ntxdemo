
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

4. **与 Harbor（OCI Registry）入库流程集成（推荐：B 方案）**
	- 平台从 Harbor 拉取 `component.wasm`（OCI artifact layer）
	- 以 `wasm sha256` 作为唯一版本/主键：生成并落库 `catalog_json`，保证 catalog 与 wasm 强一致
	- 可选：把 `actions-catalog.json` 作为第二个 layer 一起发布，用于离线分发/快速预览；但仍推荐平台以 `wasm_sha256` 为准做校验或重算

## 与 Harbor 入库流程集成

目标：平台从 Harbor 接收/拉取 `component.wasm` 后，在入库流程中调用本工具生成 catalog，并与 wasm 以同一主键落库，避免“catalog 与 wasm 不匹配”。

### 约定（建议）

- OCI Artifact layers：
	- `component.wasm`（必须）
	- `actions-catalog.json`（可选）
- 主键：`wasm_sha256 = sha256(component.wasm bytes)`
- 入库强一致策略（推荐其一）：
	- **生成式（推荐）**：忽略远端 catalog layer（即使存在），平台总是基于拉取到的 wasm 运行 `actions-catalog-gen` 生成并落库
	- **校验式**：若 artifact 同时包含 `actions-catalog.json` layer，则平台用同一个 wasm 生成一份 catalog，与 layer 内容做字节级/哈希比对，通过才入库

### 可执行入库步骤（平台侧）

1) 从 Harbor 拉取 artifact，拿到 `component.wasm`

```bash
oras pull <harbor-host>/<project>/<repo>:<version> -o <out-dir>
```

2) 计算 `wasm_sha256`（主键/去重 key）

```bash
sha256sum <out-dir>/component.wasm
```

3) 运行 generator 生成 catalog（建议写到临时目录或直接 stdout 管道）

```bash
cargo run -p actions-catalog-gen -- <out-dir>/component.wasm > <out-dir>/actions-catalog.json
```

4) 开启 DB 事务（示意）：
	- upsert `wasm_artifacts(wasm_sha256, wasm_bytes, wasm_size, ...)`
	- upsert `wasm_catalogs(wasm_sha256, schema_version, catalog_json, generated_at, ...)`

5)（可选）建立 tag->digest/sha 映射表，便于按 `<version>` 查询、回滚与审计

## 可选：把 catalog 作为第二个 layer 一起发布

适用：需要“离线分发时同时带上可读的 catalog”、或者想让非平台侧工具快速查看 action 列表。

建议仍以 wasm 作为真相源：平台入库时重算或校验 catalog，以保证强一致。

### ORAS push 示例（WASM + Catalog 双 layer）

```bash
oras push <harbor-host>/<project>/<repo>:<version> \
	--artifact-type application/vnd.ntx.action-executor.v1 \
	component.wasm:application/wasm \
	actions-catalog.json:application/json
```

说明：

- 文件名（`component.wasm` / `actions-catalog.json`）建议固定，便于平台侧 pull 后按约定读取。
- 内容类型（media type）可按你们治理策略细化（例如为 catalog 定义 `application/vnd.ntx.actions-catalog.v1+json`），但最小可用用 `application/json`。

