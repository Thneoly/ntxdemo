# MVP 方案（进度管理系统）

**目标**：在最短 2–3 周内交付一个可验证的最小可行版本，证明“稀疏 Workbook + progress 事件总线 + 资源健康联动”在 `http_tri_phase_demo` 场景下可稳定运行，并达到 Phase 1 的核心验收指标。

## 1. 范围与取舍

| 包含 | 说明 |
| --- | --- |
| Runner Core 稀疏状态存储 | 仅实现脏字段跟踪 + HashMap 按需存储，明确聚焦 `timeline.phase`、`execution.progress`、`governance.status` 三个关键字段 |
| progress.batching 参数 | `max-batch-size`、`flush-interval`、`priority-threshold` 挂载在 DSL，并由 Runner Core 执行 |
| progress 事件总线 | 替代轮询检查的核心骨架，供 Poller/Telemetry 订阅 |
| 进度级联与聚合 | 跑通任务级→用户级的加权聚合（加权平均），DSL 支持 `progress-aggregation` 配置 |
| DSL Parser 扩展 | `resource_lease` 字段注入、progress.* schema 校验、parse2wbs 条件 AST 缓存 |
| Poller 事件接入 | 新增“进度检查”事件类型，消费 progress bus（不含 progress_pollable） |
| Resource Manager 健康检查 | `health-check` 配置执行、progress-aware release 钩子 |
| Telemetry/Observability | Prometheus 1 s 采样的 workbook 内存 / health 指标；压测仪表板 |
| QA/CI 保障 | 稀疏存储单测、progress.batching 集成测试、resource_lease 生命周期测试、`wac validate`、`get_problems` |

| 不包含 | 推迟到 Phase 2/3 |
| --- | --- |
| progress_pollable 资源、优先级队列与 WASI 异步整合 |
| 版本化状态容器、RefCell 深度优化、状态机执行器重构 |
| 资源优先级继承、预测算法、动态再分配 |
| 多维度进度健康度、采样策略、二进制 DSL、增量加载 |

## 2. 模块与交付物

### Runner Core
- 实现轻量稀疏存储（脏字段跟踪 + HashMap 高频字段拆分），MVP 暂不交付快照压缩接口。
- 落地 progress.batching 执行与任务级→用户级进度聚合（加权平均），并输出错误分类→rollback 逻辑。
- 暴露 progress bus API 给 Poller/Telemetry，并产出 `progress.wit` 初版。
- **交付**：稀疏 PoC 数据、batching/aggregation 配置指南、500 用户压测数据包。

### DSL Parser
- 注入 `resource_lease`、progress.* schema 校验，生成条件 AST。
- 支持 `progress-aggregation` 配置（加权平均）并与 `progress.wit` 接口保持一致。
- **交付**：更新后的 schema/IDL、AST 缓存实现说明、aggregation 配置示例。

### Poller (`polling.rs`)
- 订阅 Runner Core progress bus，使用 wasmtime 异步通道 + bounded queue 处理进度事件，并输出事件延迟指标。
- **交付**：进度事件类型实现、延迟监控样例日志。

### Resource Manager
- 执行 health-check 配置，on-failure 时触发 `progress.rollback → last-stable-point` 与 `resource.release`。
- **交付**：health-check 行为说明、rollback 触发日志样例。

### Telemetry / Ops
- Prometheus 指标采集（workbook 内存、health-score、事件吞吐），构建压测仪表板。
- **交付**：Grafana 面板配置、压测报告。

### Test / CI
- 单测：稀疏存储、resource_lease 生命周期。
- 集成测试：progress.batching、生效的 health-check + rollback、事件驱动进度更新。
- CI：`wac validate`、`get_problems`、压力脚本。

## 3. 执行节奏（建议）

1. **第 1 周：核心模型冻结**
   - 先完成 Runner Core 轻量稀疏模型与进度聚合设计，编写 `progress.wit` 接口草案。
   - 仅对 progress bus 核心接口进行评审；Resource Manager 确认 health-check 基础行为。
   - 产出 timeline Section PoC（100 用户、内存降低 ≥15%）。
2. **第 2 周：核心实现 + 单测**
   - 优先实现 Runner Core 脏字段跟踪、progress.batching、聚合与错误分类；DSL Parser 接口落地；Poller/Resource 交付简化版本。
   - QA 聚焦稀疏 PoC 单测、aggregation/batching 集测、`wac validate` 接口验证。
3. **第 3 周：集成验证 + 压测**
   - 使用简化版 `http_tri_phase_demo` 进行 500 用户压测，验证进度更新、资源释放与事件延迟 (<50ms)。
   - 汇总交付物：设计文档、配置指南、压测报告、Prometheus + jemalloc 数据。

## 4. 验证标准

- `get_problems` 和 `wac validate` 全部通过；CI green。
- `jemalloc` 的 `malloc_stats_print` 配合 Prometheus 曲线显示：在 500 用户规模下、场景运行稳定后 5 分钟平均值较旧模型下降 ≥20%，并附同场景基准对比。
- Resource health-check 触发时，Workbook `execution.resource_lease` 与资源回收日志保持一致。
- 进度事件总线驱动 Poller，集成测试中无需轮询即可更新 `timeline.phase`，事件延迟 <50ms。
- `http_tri_phase_demo` 用户级进度输出符合 `progress-aggregation` 权重配置。

## 5. 风险与应对

| 风险 | 影响 | 应对 |
| --- | --- | --- |
| 稀疏模型实现复杂 | 可能拖慢 MVP 节奏 | 第 1 周完成 timeline Section PoC（100 用户降低 ≥15%），若未达标则回退到 `Cow` 渐进式方案 |
| health-check 依赖组件可用性 | 压测无法覆盖异常路径 | 提供 `http.health` mock 组件，模拟超时/失败 |
| progress bus 与 Poller 同步问题 | 可能出现事件堆积 | 使用 wasmtime 异步通道 + bounded queue，并在 Telemetry 中监控事件延迟 (<50ms) |
| 进度聚合准确性不足 | 可能无法验证核心价值 | 对 `progress-aggregation` 添加权重校验测试，压测期间实时抽样验证 |

## 6. 下一步

- 依据本方案创建模块级 Issue，并绑定责任人。
- 在 `docs/plan/templates/`（可选）放置交付物模板（压测报告、Prometheus 导出）。
- MVP 完成后立即切入 Phase 2：progress_pollable、版本化状态与 WASI 深度适配。
