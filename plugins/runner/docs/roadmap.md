# 进度管理系统实现路标

本路标遵循`进度管理系统设计与WASI优化规范`和`WebAssembly组件接口与集成规范`，采用三阶段实施策略，确保与现有系统兼容的同时实现进度管理能力的全面提升。

## 实施原则

- **向后兼容**：确保现有workflow不受影响，新旧进度模型可并行运行
- **渐进式切换**：通过特性开关控制新功能启用
- **规范遵循**：严格遵守WASI环境优化规范和组件接口规范
- **验证驱动**：每个功能点都包含明确的验证标准

## Phase 1：基础优化（核心进度模型）

**目标**：实现进度模型基础框架，解决内存效率和状态转换问题
**时间框架**：2-4周
**验证标准**：通过`get_problems`验证无规范冲突，1000用户规模下内存占用降低30%（基线采集：使用Prometheus监控workbook内存指标，采样间隔1s）

### 必须完成事项

#### 进度模型基础
- [ ] 实现workbook稀疏状态存储：分离高频/低频字段，添加"脏字段"跟踪机制 (模块：Runner Core)
- [ ] 添加状态快照压缩接口，定期压缩稳定状态 (模块：Runner Core)
- [ ] 在`http_scenario.yaml`中定义`resource_lease`字段，建立资源与进度的绑定 (模块：DSL Parser)
- [ ] 实现progress.batching参数：`max-batch-size`、`flush-interval`、`priority-threshold` (模块：Runner Core)
- [ ] 状态条件预编译：在parse2wbs阶段生成AST并缓存，支持`{{...}}`条件表达式 (模块：DSL Parser，依赖：稀疏状态存储)

#### 进度事件与资源
- [ ] 实现进度事件总线基础框架，替代轮询检查 (模块：Runner Core)
- [ ] 在`polling.rs`中添加进度检查事件类型 (模块：Poller，依赖：稀疏状态存储)
- [ ] 为resource-manager添加进度感知释放钩子 (模块：Resource Manager)
- [ ] 在ip-port-pool中实现health-check配置块，支持interval、timeout和on-failure处理 (模块：Resource Manager)

#### 验证与质量保证
- [ ] 添加workbook稀疏存储的单元测试，验证内存效率 (模块：Test Framework)
- [ ] 实现progress.batching参数的集成测试 (模块：Test Framework)
- [ ] 验证resource_lease与资源生命周期的正确关联 (模块：Test Framework)
- [ ] 通过`wac validate`验证进度相关接口一致性 (模块：CI Pipeline)

### 阶段交付物
- 稀疏状态存储设计文档
- progress.batching参数配置指南
- 1000用户压力测试报告
- 内存优化效果对比数据包（包含基线与优化后数据）

## Phase 2：WASI适配（环境集成）

**目标**：确保进度系统与WASI环境深度适配，符合单线程优化规范
**时间框架**：3-5周
**验证标准**：通过WASI规范验证，单线程性能提升20%（基线采集：使用wasi-sdk性能分析工具，采样间隔100ms），无RefCell借用冲突

### 必须完成事项

#### WASI Poller集成
- [ ] 在`polling.rs`中实现专用`progress_pollable`资源 (模块：Poller)
- [ ] 确保每次轮询迭代重建future实例，避免卡顿 (模块：Poller，依赖：progress_pollable)
- [ ] 实现进度检查优先级队列，支持基于`progress.poller.priorities`的调度 (模块：Poller)
- [ ] 添加progress polling性能监控指标 (模块：Telemetry)

#### 状态管理优化
- [ ] 实现"版本化状态"机制，减少RefCell嵌套 (模块：Runner Core，依赖：稀疏状态存储)
- [ ] 开发状态合并策略，批量处理小更新 (模块：Runner Core)
- [ ] 重构进度检查逻辑，避免深度嵌套RefCell (模块：Runner Core)
- [ ] 实现进度数据的轻量级状态机替代动态表达式求值 (模块：Runner Core，依赖：状态条件预编译)

#### 资源-进度深度联动
- [ ] 在资源健康检查失败时触发`progress.rollback → last-stable-point` (模块：Resource Manager，依赖：health-check)
- [ ] 实现资源租约与进度状态的双向绑定 (模块：Resource Manager)
- [ ] 添加资源使用预测，基于进度预测提前准备资源 (模块：Resource Manager)
- [ ] 实现资源优先级继承，从关联进度继承优先级 (模块：Resource Manager)

#### 验证与质量保证
- [ ] 验证进度检查与Poller集成符合`WASI环境下异步选择器实现规范` (模块：CI Pipeline)
- [ ] 通过压力测试验证单线程性能提升 (模块：Test Framework)
- [ ] 验证无RefCell借用冲突，特别是高并发场景 (模块：Test Framework)
- [ ] 确保组件接口通过`wac validate`验证 (模块：CI Pipeline)

### 阶段交付物
- progress_pollable设计与API文档
- WASI环境适配验证报告
- 单线程性能测试结果（包含基线与优化后数据）
- RefCell借用冲突排查指南

## Phase 3：高级功能（智能调度）

**目标**：添加高级进度管理能力，提升系统智能化水平
**时间框架**：4-6周
**验证标准**：进度预测准确率>85%（基线采集：使用历史测试数据验证，采样窗口7天），资源利用率提升25%（基线采集：通过Prometheus监控资源使用率，采样间隔5s）

### 必须完成事项

#### 多维度进度度量
- [ ] 在telemetry中扩展工作量进度计数器 (模块：Telemetry)
- [ ] 在`load.scenarios`中实现里程碑节点定义 (模块：DSL Parser，依赖：telemetry扩展)
- [ ] 开发综合进度健康度评分体系：`health = (progress * 0.6) + (1 - error-rate) * 0.3 + (1 - deviation) * 0.1` (模块：Runner Core)
- [ ] 实现进度采样策略：`full`、`sampled`、`aggregated`三种模式 (模块：Runner Core)

#### PMP框架深度整合
- [ ] 扩展timeline section至完整五大过程组：`initiating`、`planning`、`executing`、`monitoring`、`closing` (模块：DSL Parser)
- [ ] 在registers中引入干系人管理条目 (模块：DSL Parser)
- [ ] 实现十五至尊图进度映射，关联范围、时间、成本等维度 (模块：Runner Core，依赖：五大过程组)
- [ ] 定义过程组级进度指标，如`stakeholders-identified`、`wbs-completed`等 (模块：Runner Core)

#### 智能调度与优化
- [ ] 实现基于历史数据的进度预测算法（指数平滑） (模块：Runner Core，依赖：telemetry扩展)
- [ ] 开发资源动态再分配机制，基于进度状态自动调整 (模块：Resource Manager，依赖：进度预测)
- [ ] 实现优先级动态重计算，结合进度健康度和业务价值 (模块：Runner Core)
- [ ] 添加错误影响分析，评估错误对整体进度的影响 (模块：Runner Core)

#### DSL性能优化
- [ ] 实现二进制DSL格式，在构建阶段将YAML编译为紧凑二进制 (模块：DSL Parser，依赖：parse2wbs)
- [ ] 添加DSL缓存机制，避免重复解析 (模块：DSL Parser)
- [ ] 引入增量DSL加载，仅加载当前场景所需部分 (模块：DSL Parser)
- [ ] 实现DSL编译时验证，确保类型安全 (模块：DSL Parser)

#### 验证与质量保证
- [ ] 验证进度预测算法的准确率 (模块：Test Framework)
- [ ] 测试资源动态再分配对系统性能的影响 (模块：Test Framework)
- [ ] 验证PMP框架映射的完整性 (模块：Test Framework)
- [ ] 确保二进制DSL与YAML DSL的向后兼容性 (模块：Test Framework)

### 阶段交付物
- 多维度进度度量API文档
- 进度预测算法验证报告
- PMP框架映射指南
- 二进制DSL格式规范

## 风险控制与保障措施

### 兼容性保障
- [ ] 为所有新功能添加特性开关，默认关闭 (模块：Feature Flag System)
- [ ] 实现新旧进度模型并行运行机制 (模块：Runner Core)
- [ ] 提供进度模型迁移工具 (模块：Migration Tools)
- [ ] 编写详细的迁移指南 (模块：Documentation)

### 性能保障
- [ ] 建立性能基线，定期对比验证 (模块：Telemetry)
- [ ] 实现性能监控仪表板 (模块：Dashboard)
- [ ] 设置性能阈值告警 (模块：Telemetry)
- [ ] 制定性能回滚预案 (模块：Operations)

### 质量保障
- [ ] 每个功能点必须有明确的验证标准 (模块：QA Process)
- [ ] 采用`get_problems`验证代码规范符合性 (模块：CI Pipeline)
- [ ] 通过`wac validate`验证组件接口一致性 (模块：CI Pipeline)
- [ ] 建立自动化测试套件，覆盖关键场景 (模块：Test Framework)
