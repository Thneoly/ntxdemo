# Feature Flag Strategy

## Goals
- Gate risky Runner features (wasm polling, sparse store, progress bus) per environment.
- Support CLI overrides for smoke tests.
- Ensure telemetry reports which flags are enabled.

## Flag Table
| Flag | Default | Description | Owner |
| --- | --- | --- | --- |
| `runner.wasm_polling` | `false` | Enables async poller in `polling.rs`. | Runtime |
| `runner.sparse_store` | `true` | Activates HashMap-backed sparse workbook store. | Data |
| `runner.progress_bus` | `true` | Publishes progress events to `progress.wit`. | Core |
| `runner.http_metrics` | `false` | Emits HTTP span metrics; depends on Prometheus. | Ops |

## Storage & Evaluation
- Source of truth: `runner/res/feature_flags.toml` committed per environment.
- Runtime loads TOML once at startup; hot reload every 60s via `res` watcher.
- CLI overrides: `runner run --flag runner.wasm_polling=true` → merges into in-memory map (highest precedence).
- Env overrides: `NTX_FLAG_runner__progress_bus=false` → parsed on boot (second precedence).

### Example TOML
```toml
[default]
runner.sparse_store = true
runner.progress_bus = true

[staging]
runner.wasm_polling = true

[prod]
runner.http_metrics = true
```

## Instrumentation
- `progress::heartbeat` includes active flag set in payload metadata.
- Telemetry export adds label `flag_runner_wasm_polling="true"` etc.

## Rollout Process
1. Add new flag entry to table + TOML with default off.
2. Land code guarded by `FeatureFlags::is_enabled("runner.foo")`.
3. Run smoke tests with `--flag runner.foo=true`.
4. Flip in staging TOML, monitor dashboards 24h.
5. Promote to prod if error rate <1% and memory delta <5%. 
