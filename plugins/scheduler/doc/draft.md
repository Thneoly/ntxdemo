# Scheduler Draft

## 流程目标
1. **parse config**：解析 DSL（YAML / WIT）场景描述。
2. **build WBSTree**：把 workflow 转成静态 Work Breakdown Structure，明确定义资源、任务和依赖。
3. **build StateMachine**：根据 WBSTree 构建 FSM 拓扑，保留节点、边和状态元信息。
4. **bind triggers**：把 DSL 中的 condition/trigger 绑定到 FSM 边，得到可执行的 Runtime FSM。
5. **record Workbook**：runtime 将执行痕迹写入 Workbook，提供回放、统计、数据共享。

## 核心概念
### DSL Scenario
- `workbook.resources`：描述 http_endpoint、tcp_endpoint 等资源模板。
- `actions.actions`：可执行的原子操作（REST 调度、脚本、组件调用）。
- `workflows.nodes`：由 action / end 节点组成的图，节点之间通过带条件的 edge 串联。

### WBSTree（静态）
> 表示任务和资源的映射关系，描述“需要做什么”。

```rust
pub struct WbsTree {
        pub name: String,
        pub resources: IndexMap<ResourceId, ResourceSpec>,
        pub tasks: IndexMap<NodeId, WbsTask>,
}

pub struct WbsTask {
        pub action_id: Option<String>,
        pub kind: WbsTaskKind,   // action / end / decision
        pub outgoing: Vec<WbsEdge>,
}
```

构建策略：解析 workflow.nodes，逐个节点生成 `WbsTask`，并记录资源引用、导出变量需求。

### StateMachine（状态）
> 表示“如何运行”——FSM 的节点、状态、上下文投影。

```rust
pub struct StateMachine {
        pub nodes: IndexMap<NodeId, StateNode>,
}

pub struct StateNode {
        pub task: Arc<WbsTask>,
        pub transitions: Vec<StateTransition>,
}
```

### Trigger（动态）
- DSL `trigger.condition` 字符串保持原样；后续可考虑 DSL → AST → 可执行条件。
- 触发器绑定在 `StateTransition` 上，`Always` 代表无条件流转。

### Workbook（运行记录）
- 记录资源绑定值、action 执行结果、状态轨迹。
- 读取场景中 `export` 定义自动注册指标，便于统计、回放。

## 数据流与分层

```
                         (Static)
                  ┌──────────────┐
                  │   WBSTree    │  ← DSL 解析得到
                  └───────┬──────┘
                         build FSM
                                  ▼
                  ┌─────────────────────┐
                  │   StateMachine      │  ← 可执行图结构
                  └───────┬────────────┘
                         bind triggers
                                  ▼
                  ┌─────────────────────┐
                  │  Executable FSM     │  ← 动态 Runtime
                  └───────┬────────────┘
                 runtime writes to Workbook
                                  ▼
                  ┌─────────────────────┐
                  │     Workbook        │  ← 回放/统计/共享
                  └─────────────────────┘

## 调用逻辑路线

Runtime 由 `SchedulerPipeline::run` 驱动，整体分为三个阶段：

1. **`init`**：注入的 `ActionComponent` 先执行 `init()`，可在此处创建连接池、加载配置或完成鉴权。
2. **`do_action`**：
        - Pipeline 将 WBSTree 中的 action task 排成队列。
        - 对每个 task：解析 action → 构造 `ActionContext` → 调用 `component.do_action(action, ctx)`。
        - `ActionContext` 提供 CRUD 能力，可在执行过程中注册新 action、增删任务或更新 edge。
3. **`release`**：所有任务完成后调用 `component.release()`，用于 flush、关闭句柄或回传统计。

> 任意在 `ActionContext` 上的修改都会同步 StateMachine，因此新增/修改节点立即可见。

### 动态扩展场景示例

以 `probe-get → push-post → end` 为例：

1. `component.init()` 打开 HTTP 客户端并缓存目标地址。
2. 执行 `probe-get`：若返回结果需要额外推送，调用 `ctx.add_task` 新增 `dynamic-node`，并设置指向 `end` 的 edge（带 label `dynamic`）。
3. Scheduler 发现新的 task ID，会把它加入队列并继续调用 `do_action`，确保动态插入的任务在同一轮次被执行。
4. 所有任务完成后，`component.release()` 关闭客户端、写入 Workbook。

这种循环保证了“运行期策略决定后续拓扑”的能力，无需提前在 DSL 中枚举所有分支。
```

## 近期实现清单（MVP）
1. **Parser**：`Scenario::from_yaml` + 基础校验（节点引用、action 是否存在）。
2. **WBSTree Builder**：生成任务树、资源索引、导出项元数据。
4. **Runtime Skeleton**：提供 `ActionComponent` Trait（`init → do_action → release`），通过 `SchedulerPipeline::run` 注入，实现 action 执行 + WBS/FSM 动态更新。
4. **Runtime Skeleton**：提供 `ActionExecutor` Trait，通过 `SchedulerPipeline::run` 注入，实现 action 执行 + WBS/FSM 动态更新。
5. **WBSTree/StateMachine CRUD**：执行过程中支持新增/删除/修改任务以及 edge，`ActionContext` 同步更新 FSM。
6. **Workbook Store**：基于 `IndexMap<String, serde_yaml::Value>` 的轻量实现，后续可替换为 cap-std/fs backend。

## 验收准则
- 支持加载 `res/http_scenario.yaml`，打印资源、节点数量。
- FSM 结构可序列化 / debug 输出，执行时可动态增删 task。
- Workbook 能记录 action 输入/输出雏形。
- `ActionComponent` demo 能遍历所有 action，并允许在运行期插入新的 task。

## 开放问题
- Trigger 表达式语言：临时保留字符串，后续接入 DSL AST。
- Action 执行器：需要整合 runner/runtime，暂以占位符 `ActionComponent` 表示。
- 并发/超时策略：MVP 仅支持串行执行，后续引入 scheduler。
