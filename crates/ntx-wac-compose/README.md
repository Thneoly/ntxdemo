# ntx-wac-compose

A tiny Rust wrapper that runs `wac compose` via `std::process::Command`.

## Usage

From repo root:

```bash
cargo run -p ntx-wac-compose -- \
  --wac-file component/wac/scheduler-composition.wac \
  --deps-dir component/wac/deps \
  -o component/wac/scheduler-composed.wasm
```

Override `wac` path:

```bash
cargo run -p ntx-wac-compose -- --wac-bin /path/to/wac
```

Forward extra args to `wac compose`:

```bash
cargo run -p ntx-wac-compose -- -- --verbose
```
