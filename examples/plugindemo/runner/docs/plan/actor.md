# Runner Actor Plan

> 目的：将尚需补足的开发前提（接口、基线、测试、构建、数据模型、特性开关）串成可执行的行动列表，确保新成员加入即可推进实现。

## 任务与责任

| 编号 | 任务 | 产出 | 责任模块 | 前置依赖 | 预计完成 |
| --- | --- | --- | --- | --- | --- |
| A1 | 定义 `progress.wit` 接口 | `docs/wit/progress.wit`（含事件、错误、订阅 API）+ 对应说明 | Runner Core + DSL Parser | Roadmap Phase 1 需求 | Week 1 Day 2 |
| A2 | 编写基线采集指南 | `docs/plan/baseline.md`：jemalloc `malloc_stats_print` 步骤、Prometheus 查询脚本、旧模型对比表 | Telemetry/Ops | A1（需接口确定以便指标挂接） | Week 1 Day 3 |
| A3 | 测试/工具链策略 | `docs/testing/strategy.md`：unit（cargo test + mock）、integration（wasmtime + scenario）、CI 命令清单 | QA + Dev | A1（接口 stub）、README 环境部分 | Week 1 Day 4 |
| A4 | 构建与运行说明 | 扩展 `README.md` + `docs/plan/runbook.md`：Runner Core 构建、plugins/*/build.sh、run.sh 调用流程、所需 env | Dev Enablement | 现有 README | Week 1 Day 4 |
| A5 | 稀疏存储数据模型细化 | `docs/plan/sparse_model.md`：HashMap key schema、版本号策略、序列化格式、Cow fallback | Runner Core | A1（字段命名） | Week 1 Day 5 |
| A6 | 特性开关策略 | `docs/plan/feature_flags.md`：flag 名称、存储位置（config/env）、启用流程、灰度策略 | Runner Core + Ops | A4（运行说明） | Week 1 Day 5 |

## 执行节奏

1. **Week 1 Day 1-2**
   - 完成 `progress.wit` 初稿与评审（A1）。
   - 同步 DSL Parser / Poller / Resource Manager 接口依赖。
2. **Week 1 Day 3-4**
   - Telemetry 团队依据接口输出基线采集指南（A2）。
   - QA/Dev 共拟测试策略（A3），同时补 README/Runbook（A4）。
3. **Week 1 Day 5**
   - Runner Core 产出稀疏存储数据模型细节（A5），并串联 feature flag 策略（A6）。

## 验收清单

- [ ] progress.wit PR 合并并通过 `wac validate`。
- [ ] baseline 指南包含旧模型 vs MVP 表格与脚本示例。
- [ ] testing/strategy.md 明确命令、mock、CI 触发方式。
- [ ] README/Runbook 覆盖 Runner 构建运行与 build.sh/run.sh 引用。
- [ ] sparse_model.md 给出 Key/Version/Serialize 决策与 Cow fallback。
- [ ] feature_flags.md 列出 flag 名称（如 `runner.progress.migrate`）、注入方式、灰度开关流程。

## 风险与缓解

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| progress.wit 设计延迟 | 其他模块无法并行 | Day 2 前完成评审，必要时先给 Alpha 版接口 + 兼容层 |
| 基线数据缺少旧模型样本 | 内存对比不可验 | 在指南中附上采集脚本 + 示例 JSON，要求每次改动先跑旧模型 |
| 测试环境未统一 | CI Flaky | strategy.md 明确工具链 + env，CI 引入固定容器镜像 |
| 特性开关失控 | 功能上线风险 | feature_flags.md 规定默认关闭、配置文件存储及监控钩子 |
