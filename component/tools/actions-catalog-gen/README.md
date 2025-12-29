
# actions-catalog-gen

Generate an **Actions Catalog JSON** by instantiating an `actions-executor` **WASIp2 component** and calling its self-describing APIs:

- `schema-version()`
- `list-actions()`
- `describe-action(action-id)`

This is the “no manifest” path: the executable component is the source of truth for the action list and action schemas.

## Requirements

- Rust toolchain for this repo
- The `wasm32-wasip2` target installed

## Build the actions-executor component

From the repo root:

```bash
cargo build -p actions-executor --target wasm32-wasip2
```

The output component is expected at:

- `target/wasm32-wasip2/debug/actions_executor.wasm`

(If you build `--release`, adjust the path accordingly.)

## Build and run the generator

### Print catalog JSON to stdout

```bash
cargo run -p actions-catalog-gen -- target/wasm32-wasip2/debug/actions_executor.wasm
```

### Write catalog JSON to a file

```bash
cargo run -p actions-catalog-gen -- \
	target/wasm32-wasip2/debug/actions_executor.wasm \
	component/conf/udp-echo-minimal/actions-catalog.json
```

The second argument is optional. If omitted, the JSON is written to stdout.

## Output format

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

Notes:

- `params-schema-json` and `default-params-json` are **JSON strings** (so the frontend can `JSON.parse` them).
- `capabilities` is currently emitted as a list of debug strings to avoid coupling this tool to WIT field renames.

## Troubleshooting

### "file not found" / wrong component path

Make sure you built the component with the same profile/target you’re referencing.

- debug build: `target/wasm32-wasip2/debug/actions_executor.wasm`
- release build: `target/wasm32-wasip2/release/actions_executor.wasm`

### WASI / import linking errors

This tool wires a minimal no-op implementation of the `event-bus` import so the component can instantiate.
If the actions-executor world adds new imports later, this tool may need to add matching host stubs.

