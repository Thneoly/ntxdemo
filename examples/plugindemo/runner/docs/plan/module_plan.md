# 模块化设计与开发计划

**来源**：本计划综合 `docs/roadmap.md` 与 `docs/draft.md`，以三阶段（基础优化 → WASI 适配 → 高级功能）为主线，将关键模块的设计与开发工作拆解为可执行的迭代项，并给出依赖关系、交付物与验证方式。

## 1. 横向里程碑概览

| Phase | 目标 | 核心模块 | 主要交付物 |
| --- | --- | --- | --- |
| Phase 1：基础优化 (2-4 周) | 建立稀疏进度模型、事件驱动骨架 | Runner Core、DSL Parser、Resource Manager、Poller（事件接入） | 稀疏状态设计文档、progress.batching 指南、资源健康检查方案、1000 用户压测数据 |
| Phase 2：WASI 适配 (3-5 周) | 深度整合 progress_pollable、版本化状态、单线程优化 | Poller、Runner Core、Resource Manager、Telemetry | progress_pollable API、WASI 适配报告、性能对比数据、RefCell 排查指南 |
| Phase 3：高级功能 (4-6 周) | 智能调度、PMP 深度映射、DSL 性能优化 | Runner Core、DSL Parser、Telemetry、Resource Manager | 多维度度量 API、预测算法报告、PMP 映射指南、二进制 DSL 规范 |

## 2. 模块级设计与开发计划

### 2.1 Runner Core

**Phase 1**
- 实现 Workbook 稀疏存储（高频缓存 + 低频快照）与脏字段跟踪。
- 提供状态快照压缩接口与 progress.batching 参数（max-batch-size、flush-interval、priority-threshold）。
- 构建 progress 事件总线骨架，暴露订阅 API 供 Poller/Telemetry 使用。
- 交付：稀疏存储设计文档、batching 配置指南、内存压测数据。

**Phase 2**
- 引入“版本化状态”容器与更新批处理策略，替换深层 RefCell。
- 接入 Progress Pollable 的优先级调度结果，保证关键阶段优先刷新。
- 重构进度检查逻辑，提供轻量状态机执行器（依赖 DSL 预编译 AST）。
- 交付：WASI 适配报告中 Runner Core 章节、RefCell 借用排查记录。

**Phase 3**
- 扩展多维度进度指标计算、健康度评分、进度采样策略（full/sampled/aggregated）。
- 实现指数平滑预测算法、优先级动态重计算、错误影响分析。
- 输出 PMP “十五至尊图”映射实现（过程组指标、WBS 关联）。
- 交付：多维度度量 API 文档、预测算法验证报告、PMP 映射指南。

### 2.2 DSL Parser

**Phase 1**
- 在 `http_scenario.yaml` 等脚本中接入 `resource_lease`、progress.batching 配置。
- parse2wbs 阶段生成条件表达式 AST 并缓存，为 Runner Core 及 Phase 2 状态机准备。

**Phase 2**
- 提供 progress.poller.priorities、timeline 全过程组、registers 干系人条目等 schema 校验。
- 输出版本化 DSL schema 及 IDE 校验规则；配合 `wac validate` pipeline。

**Phase 3**
- 扩展 telemetry/workbook schema 以支持里程碑节点、过程组指标。
- 实现二进制 DSL 编译、缓存与增量加载，以及编译期类型校验。
- 交付：二进制 DSL 规范、IDL/schema 更新、增量加载设计。

### 2.3 Poller（`polling.rs`）

**Phase 1**
- 接入 Runner Core progress 总线，先行注册“进度检查”事件类型（依赖稀疏存储）。

**Phase 2**
- 实现专用 `progress_pollable` 资源，确保每次迭代重建 future。
- 支持 `progress.poller.priorities`（phase→weight）优先级队列与性能监控指标。
- 纳入 WASI 异步选择器验证，输出 API 文档。

**Phase 3**
- 结合 Runner Core 的预测结果调整 Poller 执行优先级；支持动态权重刷新。
- 对接 Telemetry，提供 Pollable 延迟、队列深度指标。

### 2.4 Resource Manager

**Phase 1**
- 增加 `health-check` 配置（interval/timeout/probe/on-failure）并实现执行路径。
- 提供 progress-aware release 钩子，确保 resource_lease 与 Workbook 状态关联。

**Phase 2**
- 在健康检查失败时触发 `progress.rollback → last-stable-point`。
- 构建资源租约与进度状态双向绑定、资源优先级继承与使用预测。

**Phase 3**
- 接入 Runner Core 的动态再分配与预测结果，自动调度资源池。
- 输出资源利用率指标，支撑 25% 提升目标。

### 2.5 Telemetry & Observability

**Phase 1**
- 与 Runner Core 协同采集 workbook 内存、事件吞吐、health-check 结果，使用 Prometheus 1s 采样。

**Phase 2**
- 增加 progress polling 性能指标（wasi-sdk 分析、100ms 采样），纳入性能仪表板。

**Phase 3**
- 扩展工作量指标、健康度得分、采样模式统计；输出资源利用率监控。
- 维护性能基线与阈值告警配置。

### 2.6 Test Framework & CI

- Phase 1：补齐稀疏存储单测、progress.batching 集成测试、resource_lease 生命周期测试；CI 中运行 `wac validate` 与 `get_problems`。
- Phase 2：新增 progress_pollable/WASI 规范验证、单线程性能压测脚本、RefCell 冲突检测。
- Phase 3：引入预测准确率验证、资源再分配性能测试、PMP 映射完整性检查、二进制 DSL 兼容测试。

### 2.7 Documentation & Operations

- 每阶段输出对应交付物（设计文档、API、测试报告、数据包），并维护迁移指南与 feature flag 手册。
- Ops 团队负责性能监控仪表板、阈值告警与回滚预案的实现与演练。

## 3. 迭代节奏建议

1. **Phase 内部节奏**：按照 1 周迭代推进，先完成模块级设计评审，再进入开发与验证。
2. **跨模块同步**：每周例会上使用 roadmap checklist 更新完成度，并同步 Runner Core/DSL/Poller 的接口变更。
3. **验收关卡**：
   - 迭代中：通过自动化测试与指标看板验证。
   - 迭代末：提交阶段交付物与对比数据，评审通过后再启用对应 feature flag。

## 4. 风险与应对

| 风险 | 影响 | 应对 |
| --- | --- | --- |
| 稀疏存储与版本化状态实现复杂 | 影响 Phase 1/2 交付 | 先在 Runner Core 建立 PoC，再扩展到全场景，并通过单测覆盖关键路径 |
| Poller 优先级调度不当导致饥饿 | 影响监控稳定性 | 在 Telemetry 中加入队列深度告警，并设置最低保底轮询频率 |
| 二进制 DSL 兼容风险 | 影响 Phase 3 发布 | 维持 YAML/二进制双通道，提供迁移工具与回滚策略 |
