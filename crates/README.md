# 框架工具

本目录包含 NTX 的辅助工具与配套组件，覆盖：action executor 开发、catalog 生成、以及基于 OCI 的组件分发。

## 1) action executor 框架

- 使用 `cargo generate` 生成 action executor 框架（略）

## 2) Actions Catalog（供前端调用）

- 使用 `actions-catalog-gen` 从 `actions-executor` 的 WASIp2 component 生成 actions catalog JSON

推荐把 **WASM component** 当作唯一真相源（source of truth）：catalog 由宿主侧解析生成，可缓存，但不建议人工维护。

## 3) 组件分发：选用 Harbor（OCI Registry）

我们选用 **Harbor** 作为自建 OCI Registry（实现标准 OCI Distribution API），不自己开发 registry。

目标：把 **WASM component + 元数据** 作为 **OCI Artifact** 发布到 Harbor，并由平台拉取后落库。

### 3.1 为什么不使用自研 registry

仓库内的 `ntx-registry` / `ntx-registry-server` / `ntx-registry-cli` 当前更偏“自定义文件分发 + by-digest 路径”，不等价于标准 OCI Registry（缺少 `/v2/*` 的 manifests/blobs API 语义）。

如果需要 OCI 标准兼容（与 `oras` 等生态工具对接），推荐直接部署 Harbor。

### 3.2 Artifact 结构（建议）

一个 `actions-executor` component（一个 `.wasm`）发布为一个 OCI Artifact：

- layer: `component.wasm`
- （可选）layer: `actions-catalog.json`
- annotations：写入用于检索/治理的元数据（例如 `io.ntx.*` 与 `org.opencontainers.image.*`）

说明：是否把 `actions-catalog.json` 作为 layer 一起发布取决于你们平台策略。
若采用 **B 方案（入库时生成 catalog）**，可以不把 catalog 随 artifact 发布。

### 3.3 发布/拉取（Harbor + ORAS 示例）

以下示例使用 `oras` 与 Harbor 对接（请把域名/项目/仓库名按实际替换）。

登录 Harbor（推荐用 Robot Account）：

```bash
oras login <harbor-host>
```

发布（push）WASM（最小）：

```bash
oras push <harbor-host>/<project>/<repo>:<version> \
	--artifact-type application/vnd.ntx.action-executor.v1 \
	component.wasm:application/wasm
```

拉取（pull）WASM：

```bash
oras pull <harbor-host>/<project>/<repo>:<version> -o <out-dir>
```

如果你们后续需要把 catalog 一起发布，把第二个 layer 加进 `oras push` 即可（内容类型与文件名可按约定调整）。

## 4) 入库方案（B）：WASM + Catalog 一起落到数据库

你们希望把 `catalog` 与 `wasm` 一起落库，并保证强一致：推荐 **B 方案**。

### 4.1 B 方案定义

- **WASM component** 是唯一真相源
- 平台在“入库流程”中：
	1) 从 Harbor 拉取/接收 `component.wasm`
	2) 计算 `sha256`（作为 content-address / 版本主键）
	3) 基于该 wasm 运行 `actions-catalog-gen` 生成 `actions-catalog.json`
	4) 以同一主键把 `wasm + catalog` 一起落库

这样可以避免“上传的 catalog 与 wasm 不匹配”，并且 catalog schema 升级后可按 `wasm_sha256` 批量重算回填。

### 4.2 建议的数据库最小模型

两表（推荐，结构清晰）：

- `wasm_artifacts`
	- `wasm_sha256` (PK)
	- `wasm_bytes` (BLOB)
	- `wasm_size`
	- `created_at`
- `wasm_catalogs`
	- `wasm_sha256` (PK/FK -> wasm_artifacts.wasm_sha256)
	- `schema_version`
	- `world`（可选）
	- `catalog_json` (JSON/BLOB)
	- `catalog_sha256`（可选）
	- `generated_at`

单表也可（小规模更省事）：`wasm_sha256`、`wasm_bytes`、`catalog_json`、`schema_version` 等合并存放。

### 4.3 入库实现步骤（可执行）

1. 从 Harbor 拉取 artifact，得到 `component.wasm`
2. 计算 `wasm_sha256`（作为主键/去重 key）
3. 调用 `actions-catalog-gen` 生成 catalog JSON
4. 开启 DB 事务：
	 - upsert `wasm_artifacts(wasm_sha256, wasm_bytes, wasm_size, ...)`
	 - upsert `wasm_catalogs(wasm_sha256, schema_version, catalog_json, ...)`
5.（可选）把 Harbor 的 `<project>/<repo>:<version>` 与 `wasm_sha256` 建立映射表（便于按 tag 查询/回滚）
