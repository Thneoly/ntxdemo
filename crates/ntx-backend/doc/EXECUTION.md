# ntx-backend 执行文档（对齐版）

本后端用于把 **前端 workflow builder**、**Harbor(OCI Registry)**、以及 **actions-catalog-gen** 的能力串起来，形成一个可落地的开发/测试/部署闭环。

当前实现目标：先提供一个“能跑、能被前端调用”的最小 HTTP 服务骨架（healthz + workflow draft 存取 + actions catalog 缓存读取）。

> 说明：Harbor 拉取 wasm、生成 catalog、入库（B 方案）会作为下一步补齐；本骨架已预留了 catalog 的缓存路径与 API 形态。

---

## 1. 运行方式（本地）

在 repo 根目录：

```bash
cargo run -p ntx-backend
```

默认监听：`127.0.0.1:8080`

可用环境变量：

- `NTX_BACKEND_BIND`：监听地址，默认 `127.0.0.1:8080`
- `NTX_BACKEND_DATA_DIR`：数据目录，默认 `./.ntx-backend`
- `NTX_BACKEND_CORS_ANY_ORIGIN`：是否允许任意来源 CORS（开发方便），默认 `true`

示例：

```bash
NTX_BACKEND_BIND=0.0.0.0:8080 \
NTX_BACKEND_DATA_DIR=./.data/ntx-backend \
cargo run -p ntx-backend
```

健康检查：

```bash
curl -sS http://127.0.0.1:8080/healthz
```

---

## 1.1 前端联调（B 阶段：先接 API）

1) 启动后端：

```bash
cargo run -p ntx-backend
```

2) 启动前端（Vite）：

```bash
cd frontend/demo-workflow

export VITE_NTX_BACKEND_URL=http://127.0.0.1:8080
export VITE_NTX_CATALOG_REF=192.168.31.138/ntx/executor:v0.0.1

npm run dev
```

验证点：

- 画布拖拽/连线后，后端 `${NTX_BACKEND_DATA_DIR}/workflows/` 会出现一个 `<id>.json`
- 重开页面会复用 localStorage 里的 workflow id（或用 `?wf=<id>` 指定加载）
- 若后端已写入 catalog，则前端会优先从后端加载；否则会显示 404（A 阶段会补齐自动生成）

---

## 2. 最小 API（当前已实现）

### 2.1 Workflow Draft（给前端保存画布 JSON）

创建/更新：

```bash
curl -sS -X POST http://127.0.0.1:8080/api/v1/workflows \
  -H 'content-type: application/json' \
  -d '{"graph": {"nodes": [], "edges": [], "viewport": {"x":0,"y":0,"zoom":1}}}'
```

读取：

```bash
curl -sS http://127.0.0.1:8080/api/v1/workflows/<id>
```

落盘位置：`${NTX_BACKEND_DATA_DIR}/workflows/<id>.json`

### 2.2 Actions Catalog（缓存读写）

写入（临时用于联调；后续会由“入库/生成流程”自动写入）：

```bash
curl -sS -X POST 'http://127.0.0.1:8080/api/v1/catalog' \
  -H 'content-type: application/json' \
  -d '{"ref":"192.168.31.138/ntx/executor:v0.0.1","catalog": {"schema-version": 1, "actions": []}}'
```

读取：

```bash
curl -sS 'http://127.0.0.1:8080/api/v1/catalog?ref=192.168.31.138/ntx/executor:v0.0.1'
```

（可选）cache miss 自动触发 ingest：

```bash
# 方式 A：单次请求开启
curl -sS 'http://127.0.0.1:8080/api/v1/catalog?ref=192.168.31.138/ntx/executor:v0.0.1&auto_ingest=true'

# 方式 B：全局开启（环境变量）
export NTX_CATALOG_AUTO_INGEST=true
```

落盘位置：`${NTX_BACKEND_DATA_DIR}/catalog/<sha256(ref)>.json`

### 2.3 Ingest（A 阶段：从 Harbor 拉 wasm 并生成 catalog）

后端会：

1) `oras pull` 把 artifact 落盘到临时目录
2) 找到 `.wasm` 文件并计算 `sha256`
3) 生成 catalog：
   - 默认：调用 `actions-catalog-gen`（wasmtime component model）即时生成
   - 可选：如果 artifact 内已携带 `actions-catalog.json`，可让后端直接复用（`prefer_published_catalog=true`）
4) 写入缓存：
   - `${NTX_BACKEND_DATA_DIR}/catalog/<sha256(ref)>.json`
   - `${NTX_BACKEND_DATA_DIR}/wasm/<sha256(wasm)>.wasm`

相关环境变量（可选）：

- `NTX_ORAS_BIN`：`oras` 可执行文件路径（默认 `oras`）
- `NTX_HARBOR_CA_FILE`：Harbor 自签 HTTPS 的 CA 文件
- `NTX_HARBOR_USER` / `NTX_HARBOR_PASS`：用于后端自动 `oras login --password-stdin`（也可以提前手动登录，然后不配这两个）
- `NTX_INGEST_KEEP_TMP=true`：保留 `${DATA_DIR}/tmp/ingest-*`（排障用）

请求示例：

```bash
curl -sS -X POST http://127.0.0.1:8080/api/v1/ingest \
  -H 'content-type: application/json' \
  -d '{"ref":"192.168.31.138/ntx/executor:v0.0.1","prefer_published_catalog":true}'
```

随后读取缓存的 catalog：

```bash
curl -sS 'http://127.0.0.1:8080/api/v1/catalog?ref=192.168.31.138/ntx/executor:v0.0.1'
```

---

## 3. 下一步要补齐的“入库/生成”（对齐 Harbor + catalog B 方案）

目标：让平台侧以 wasm 为真相源，生成并缓存 `actions-catalog.json`。

推荐流程（B 方案）：

1) 后端收到“需要某个 executor 的 catalog”的请求：
   - `ref = <harbor-host>/<project>/<repo>:<tag>`
2) 后端从 Harbor 拉取 artifact（建议用 ORAS）：
   - 自签 HTTPS：`oras pull --ca-file <harbor.crt> <ref> -o <tmpdir>`
3) 对 `component.wasm` 计算 `sha256`（作为主键/缓存键）
4) 运行 `actions-catalog-gen` 生成 catalog：
   - `cargo run -p actions-catalog-gen -- <component.wasm> <actions-catalog.json>`
5) 将 wasm + catalog 以 `wasm_sha256` 为 key upsert 到 DB（后续加 DB 模块）
6) 供前端/平台查询：
   - `GET /api/v1/catalog?ref=...` 或 `GET /api/v1/catalog?wasm_sha256=...`

证书注意事项：
- 若 Harbor 使用自签证书：ORAS 需要 `--ca-file` 或把 CA 安装到系统信任库。

### 3.1 参考脚本：用 ORAS pull/push（推荐）

仓库自带了可直接跑通的脚本（参数化后更适合复用）：

- `scripts/oras/push.sh`：构建（或指定）wasm → 生成 `actions-catalog.json` → ORAS push（双 layer）
- `scripts/oras/pull.sh`：ORAS pull → 落盘到本地目录

最小用法（在 repo 根目录）：

```bash
export HARBOR_REGISTRY=192.168.31.138
export HARBOR_REF="$HARBOR_REGISTRY/ntx/executor:v0.0.1"
export HARBOR_CA_FILE=/home/cc/Desktop/harbor/certs/harbor.crt
export HARBOR_USER=admin
export HARBOR_PASS='***'

bash scripts/oras/push.sh
bash scripts/oras/pull.sh
```

---

## 4. 部署建议（最小）

- 后端：容器化或 systemd 皆可
- 配置项以 env 为主（bind、data_dir、harbor 地址、CA 文件、凭据等）
- 前端：静态站点（Nginx）
- Harbor：按 `crates/doc/Harbor.md`
