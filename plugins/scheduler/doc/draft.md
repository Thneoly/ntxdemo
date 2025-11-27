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
        - Pipeline 将 WBSTree 中的 action task 打包成带优先级（0～63）的任务，默认 action priority = 32。
        - 对每个 task：解析 action → 构造 `ActionContext`（只读视图 + event sink）→ 调用 `component.do_action(action, ctx)`。
        - `ActionContext` 的 CRUD helper 不再直接落地，而是排队成 `SchedulerEvent`（priority = 4）交给 runtime 处理。
3. **`release`**：所有任务完成后调用 `component.release()`，用于 flush、关闭句柄或回传统计。

> Event 任务优先级更高，会在下一轮立刻执行，执行完成后才会重新扫描 WBS、同步 StateMachine。

### 优先级 / idle / 信号

- **优先级通道**：总共 64 条，0 为最高。Action task 默认 32，WBS event 采用 4，idle 占用 63。
- **idle**：当所有队列为空时自动插入 `idle` 任务，执行一次 10ms sleep，避免 CPU 空转。
- **退出机制**：Runtime 监听 `Ctrl+C`（`SIGINT`），收到后设置 shutdown flag，待当前任务执行完毕后退出循环。

### 动态扩展场景示例

以 `probe-get → push-post → end` 为例：

1. `component.init()` 打开 HTTP 客户端并缓存目标地址。
2. 执行 `probe-get`：若返回结果需要额外推送，调用 `ctx.add_task` 新增 `dynamic-node`，并设置指向 `end` 的 edge（带 label `dynamic`）。这些调用会被封装成 event task。
3. Runtime 先执行高优先级的 event，同步 WBS/FSM，再把新 task 推入 action priority 队列，继续后续 `do_action`。
4. 所有任务完成后，`component.release()` 关闭客户端、写入 Workbook。

这种循环保证了“运行期策略决定后续拓扑”的能力，无需提前在 DSL 中枚举所有分支。
```

## 近期实现清单（MVP）
1. **Parser**：`Scenario::from_yaml` + 基础校验（节点引用、action 是否存在）。
2. **WBSTree Builder**：生成任务树、资源索引、导出项元数据。
3. **StateMachine Builder**：构建 FSM 拓扑。
4. **Runtime Skeleton**：提供 `ActionComponent` Trait（`init → do_action → release`）+ 64 通道的 task loop，负责 action/event/idle 调度与 `Ctrl+C` 退出。
5. **WBSTree/StateMachine CRUD**：执行过程中支持新增/删除/修改任务以及 edge，`ActionContext` 负责封装事件，runtime 统一落地并同步 FSM。
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
