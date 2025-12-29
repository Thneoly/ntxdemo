# Ntx Workflow Demo (frontend)

A tiny frontend demo that proves the end-to-end loop:

- **actions-executor** self-describes available actions (WIT catalog API)
- host generates `actions-catalog.json`
- frontend loads the catalog, shows an **Action Palette**, and builds a workflow graph

This demo is intentionally minimal (v0). It exports a graph JSON for now.

## Prereqs

- Node.js 18+ (recommended 20+)

## Refresh the catalog

The demo reads from:

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

## Next steps (when you’re ready)

- Convert the exported graph JSON into `scenario.yaml`
- Add a parameter editor powered by `input_schema_json` / `defaults_json`
- Add node types (e.g. wait/timer/branch) and validation
