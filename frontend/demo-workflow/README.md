# Ntx Workflow Demo (frontend)

A tiny frontend demo that proves the end-to-end loop:

- **actions-executor** self-describes available actions (WIT catalog API)
- host generates `actions-catalog.json`
- frontend loads the catalog, shows an **Action Palette**, and builds a workflow graph

This demo is intentionally minimal (v0). It exports a graph JSON for now.

## Prereqs

- Node.js 18+ (recommended 20+)

## Refresh the catalog

By default, the demo reads from:

- `frontend/demo-workflow/public/actions-catalog.json`

The source-of-truth sample in this repo is:

- `component/conf/udp-echo-minimal/actions-catalog.json`

To regenerate that file (host-side generator):

```bash
cd component/conf/udp-echo-minimal
./gen-actions-catalog.sh
```

Then copy it into the demo `public/`:

```bash
cd frontend/demo-workflow
cp ../../component/conf/udp-echo-minimal/actions-catalog.json public/actions-catalog.json
```

### (推荐) 从 `ntx-backend` 读取 catalog

如果你已经运行了 `crates/ntx-backend`，也可以让前端直接从后端读取（更贴近最终联调方式）。

设置环境变量（Vite 只识别 `VITE_` 前缀）：

```bash
export VITE_NTX_BACKEND_URL=http://127.0.0.1:9090
export VITE_NTX_CATALOG_REF=192.168.31.138/ntx/executor:v0.0.1
```

然后再启动前端即可。

说明：后端读取 URL 形如：

`$VITE_NTX_BACKEND_URL/api/v1/catalog?ref=$VITE_NTX_CATALOG_REF`

当前阶段：如果后端还没写入该 ref 对应的 catalog，前端会报 404（后续 A 阶段会补齐“入库/生成”）。

## Run the demo

```bash
cd frontend/demo-workflow
npm install
npm run dev
```

Open the URL printed by Vite.

## What you can do

- Click an action in the left palette to add a node
- Connect nodes by dragging handles (React Flow default behavior)
- Copy exported graph JSON via **Copy JSON**

（联调增强）

- 前端会**自动把画布草稿保存到后端**（debounce），并把 workflow id 写入 localStorage
- 可通过 URL 参数加载后端草稿：`?wf=<id>`

## Next steps (when you’re ready)

- Convert the exported graph JSON into `scenario.yaml`
- Add a parameter editor powered by `input_schema_json` / `defaults_json`
- Add node types (e.g. wait/timer/branch) and validation
