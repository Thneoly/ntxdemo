# Baseline Capture Guide

本指南用于对比“旧模型 vs MVP 稀疏模型”在 500 用户场景下的内存与事件指标，确保验证标准可复现。

## 1. 准备
- 构建 runner：`cargo build -p runner --target wasm32-wasip2`
- 确保 `plugins/runner/run.sh` 可在本地运行并加载 `http_scenario.yaml`。
- 安装 `jemalloc` 与 `prometheus-node-exporter`（或复用集群采集）。

## 2. 运行旧模型
1. 在 `configs/runner.toml` 中关闭稀疏存储与级联特性：
   ```toml
   [features.progress]
   sparse_enabled = false
   aggregation_enabled = false
   ```
2. 启动场景（500 用户）：
   ```bash
   RUNTIME_SCENARIO=http_tri_phase_demo DISABLE_PLUGIN_BUILDS=1 ./plugins/runner/run.sh
   ```
3. 待 2 分钟预热后，在第 3 分钟执行：
   ```bash
   MALLOC_CONF=prof:true ./plugins/runner/run.sh --dump-jemalloc
   ```
   使用 `jemalloc` 的 `malloc_stats_print` 导出：
   ```bash
   jeprof --stats > baseline-old.txt
   ```
4. Prometheus 查询：
   - `sum by(instance)(workbook_memory_bytes{scenario="http_tri_phase_demo"})`
   - `avg_over_time(workbook_memory_bytes[5m])`

## 3. 运行 MVP 稀疏模型
1. 打开稀疏/聚合特性：
   ```toml
   [features.progress]
   sparse_enabled = true
   aggregation_enabled = true
   ```
2. 重复步骤 2-4，导出 `baseline-mvp.txt`。

## 4. 数据对比模板
| 指标 | 旧模型（500用户） | MVP 稀疏（500用户） | 降幅 |
| --- | --- | --- | --- |
| jemalloc active bytes (avg) |  |  |  |
| Prometheus workbook_memory_bytes (avg over 5m) |  |  |  |
| Progress bus latency p95 |  |  |  |

## 5. 提交物
- `baseline-old.txt`、`baseline-mvp.txt`
- Prometheus 导出（`promtool tsdb dump`）或截图
- 汇总表格粘贴至 `docs/plan/baseline.md` 附录。
