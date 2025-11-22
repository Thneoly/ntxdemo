# Testing & Tooling Strategy

## 1. Unit Tests
- **Framework**: `cargo test` (Rust) with `wasm32-wasip2` target for Runner Core, `wasm-bindgen-test` for wasm adapters.
- **Mocks**: Use `wit-bindgen` generated mocks for `progress.wit`; resource manager mocked via `wasmtime::component::Linker`.
- **Commands**:
  ```bash
  cargo test -p runner --lib
  cargo test -p plugins-resource-manager --target wasm32-wasip2
  ```

## 2. Integration Tests
- **Engine**: `wasmtime run` + scenario YAML harness (`plugins/runner/tests/smoke.rs`).
- **Scenarios**: `http_tri_phase_demo` (500 users), `demo_minimal` (10 users).
- **Command**:
  ```bash
  cargo test -p runner --test scenario_smoke -- --scenario http_tri_phase_demo
  ```
- **Telemetry Assertions**: Use `promtool query instant` to validate metrics thresholds.

## 3. CI Pipeline
- **Workflow**: `.github/workflows/runner.yml`
  1. `cargo fmt --check`
  2. `cargo clippy --all-targets -- -D warnings`
  3. `cargo test -p runner`
  4. `wasmtime run plugins/runner/target/wasm32-wasip2/debug/runner.wasm`
  5. `wac validate plugins/runner/target/wasm32-wasip2/debug/*.wasm`
- **Artifacts**: Upload Prometheus dumps + jemalloc reports from baseline job.

## 4. Local Tooling
- Install via `cargo binstall` (see README) + `justfile` aliases:
  - `just fmt`
  - `just lint`
  - `just scenario http_tri_phase_demo`

## 5. Responsibilities
- Runner Core team：unit tests for sparse storage、progress bus。
- QA team：integration suites + Prometheus assertions。
- Ops team：baseline job + grafana snapshots。
