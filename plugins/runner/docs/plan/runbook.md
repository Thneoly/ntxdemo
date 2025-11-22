# Runner Build & Runbook

## 环境要求
- Rust 1.76+，目标 `wasm32-wasip2`
- `cargo-component`, `wasmtime-cli`, `wit-bindgen-cli`
- jemalloc（baseline 采集）

## 构建
```bash
# 构建所有插件（默认）
cargo build -p runner --target wasm32-wasip2
```
跳过插件自动构建：
```bash
DISABLE_PLUGIN_BUILDS=1 cargo build -p runner
```
单独构建 `plugins/runner`：
```bash
cd plugins/runner
./build.sh
```

## 运行示例
```bash
cd plugins/runner
RUNTIME_SCENARIO=http_tri_phase_demo ./run.sh
```
运行并导出 jemalloc 统计：
```bash
MALLOC_CONF=prof:true RUNTIME_SCENARIO=http_tri_phase_demo ./run.sh --dump-jemalloc
```

## 配置
- `configs/runner.toml`：特性开关、资源限额。
- `plugins/runner/docs/http_scenario.yaml`：场景脚本，可通过 `RUNTIME_SCENARIO` 指定。

## 故障排查
- 查看 `target/wasm32-wasip2/debug/runner.log`。
- 若 progress.wit 接口报错，运行 `wac validate plugins/runner/wit/progress.wit`。
