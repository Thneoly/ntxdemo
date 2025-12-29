
# 前端流程编排（Workflow Builder）设计说明（参考 Dify）

本文档记录我们对 `examples/dify`（Dify Web 前端）中**流程编排画布**实现的代码级分析结论，并将其抽象成一套可复用的架构与落地方案，最终目标是为 Ntx 提供：

- 无限画布（pan/zoom）
- 拖拽添加节点
- 端口连线（edge）
- 节点/边属性编辑
- 自动布局（可选）
- 序列化/反序列化（编辑态 JSON）
- 导出运行态 DSL（Ntx：`scenario.yaml`）

> 约束对齐：我们在 `component/doc/DESIGN_PROMPT.md` 中提出“事件驱动、单线程、配置声明式”等原则；前端 Builder 只负责**生成/编辑 DSL**，不承担运行时状态机执行。

---

## 1. Dify 采用的核心前端技术栈与模块划分

### 1.1 画布引擎：React Flow

Dify 的 Workflow Builder 使用 **React Flow**（`reactflow@11.x`）。

对应主入口组件：

- `examples/dify/web/app/components/workflow/index.tsx`

关键点：

- `ReactFlow` 组件承载无限画布、交互、渲染调度
- `nodeTypes` / `edgeTypes` 注册自定义节点与边
- `useNodesState` / `useEdgesState` 管理画布态 nodes/edges
- `Viewport`（x/y/zoom）也被当作 workflow 数据的一部分参与序列化

### 1.2 数据层：Zustand Slice Store

Dify 的 workflow store 采用 **Zustand** + “Slice 拼装”模式：

- `examples/dify/web/app/components/workflow/store/workflow/index.ts`

Store 被拆成多个 slice（布局、节点交互、面板、草稿同步等），形成可维护的模块化状态树。

典型 slice：

- `layout-slice.ts`：画布尺寸、面板尺寸、最大化状态
- `node-slice.ts`：节点交互状态（候选节点、右键菜单、连接中 payload 等）
- `workflow-draft-slice.ts`：草稿缓存与 debounced 同步控制

> 设计思想：把“图数据（nodes/edges/viewport）”与“UI/交互状态（selection/panel/menu/connecting state）”分离，降低耦合。

---

## 2. Dify 如何定义节点（Node）与边（Edge）

类型定义集中在：

- `examples/dify/web/app/components/workflow/types.ts`

### 2.1 Node：React Flow Node + data 承载业务 DSL

Dify 的 Node 类型是：

- `Node<T> = ReactFlowNode<CommonNodeType<T>>`

核心字段：

- `node.id`: 节点唯一 ID
- `node.type`: **React Flow 的渲染类型**（用于匹配 `nodeTypes`）
- `node.position`: 节点坐标（编辑态）
- `node.data`: 节点业务数据（DSL + UI 辅助字段）

一个非常值得借鉴的分层：

- `node.type` 用于决定“节点外壳组件”（例如 CustomNode/NoteNode/SimpleNode）
- `node.data.type`（Dify 叫 `BlockEnum`）用于决定“节点业务语义”（Start/LLM/HttpRequest/IfElse...）

这种方式可以让我们在 Ntx 中实现：

- ReactFlow 层只有少数渲染壳（Action/Wait/End/Note）
- 具体 action_id/call/参数/事件匹配规则全都在 `node.data` 中

### 2.2 Edge：React Flow Edge + data 承载连线语义

Dify 的 Edge 类型是：

- `Edge = ReactFlowEdge<CommonEdgeType>`

在 `edge.data` 中存储：

- `sourceType/targetType`：边两端节点的业务类型
- `_hovering/_isTemp/_waitingRun`：交互/运行态渲染辅助

> 设计思想：Edge 本体只做连线，“连线是否允许/如何着色/如何显示 label/是否属于某个子图”等，都挂在 `edge.data` 上。

---

## 3. Dify 的边（Edge）如何实现：自定义渲染 + 边上插入节点

边实现参考：

- `examples/dify/web/app/components/workflow/custom-edge.tsx`

关键点：

1. 使用 `getBezierPath` 计算曲线路径；`BaseEdge` 绘制。
2. `EdgeLabelRenderer` 在边中点渲染交互控件。
3. Dify 在边上提供 `BlockSelector`：可以**直接在边中间插入一个节点**，并自动完成重连。

这一点对我们很有价值：

- 体验上比“只能拖节点到画布再连线”高效很多
- 实现上需要提供“插入节点”的原子操作：
	- 断开 `source -> target`
	- 插入 `source -> new -> target`
	- 保留原 edge 的 sourceHandle/targetHandle（分支/端口语义）

---

## 4. 节点/边的生成与校验：从 util 层抽象图操作

### 4.1 生成节点：统一工厂函数

节点创建集中在：

- `examples/dify/web/app/components/workflow/utils/node.ts`

典型做法：

- `generateNewNode(...)` 统一补齐 id/type/position/handles/层级 zIndex 等
- 对“复合节点”（Iteration/Loop）会自动生成子节点（start 节点）并绑定 parentId

> 设计思想：节点创建是 workflow 编辑器的“领域操作”，不应该散落在 UI 组件里。

### 4.2 连线与合法性：cycle 检测 / connected handles

cycle 检测、初始化 connected handles 等，集中在：

- `examples/dify/web/app/components/workflow/utils/workflow-init.ts`
- `examples/dify/web/app/components/workflow/utils/workflow.ts`

关键能力：

- `getCycleEdges(...)`：通过 DFS + color 标记检测环并定位环内 edges
- `initialNodes(...)`：初始化节点坐标、修正数据兼容、计算 `_connectedSourceHandleIds/_connectedTargetHandleIds`
- `getValidTreeNodes(...)`：从 Start/Trigger 出发遍历可达节点，形成“有效节点集合”（便于导出/校验）

> 设计思想：**图上允许临时的“未完成状态”**（例如拖拽连线中、临时边），但在导出/运行前要有一层严格的规范化与校验。

---

## 5. 自动布局：ELK（Layered）

自动布局实现：

- `examples/dify/web/app/components/workflow/utils/elk-layout.ts`

Dify 当前使用 `elkjs` 的 layered 算法（文件名虽然引用 dagre，但已迁移到 ELK）。

可以借鉴的落地方式：

- 编辑器支持“自动布局”按钮
- 仅在用户触发时运行布局（避免每次拖拽都重排造成跳动）
- 生成新 positions 后写回 nodes 并保持 viewport

---

## 6. 序列化与草稿同步：编辑态 JSON 是一等公民

### 6.1 编辑态序列化结构（推荐对齐 Dify）

Dify 的更新事件载荷里包含：

- `nodes: Node[]`
- `edges: Edge[]`
- `viewport: Viewport`

这基本就是一个可直接落库/存文件的“编辑态工程文件”。

### 6.2 草稿同步（debounce）

Dify 用 `workflow-draft-slice.ts` 提供了：

- `debouncedSyncWorkflowDraft`（5s debounce）
- `backupDraft`（可用于导入前备份、回滚）
- `flushPendingSync`（页面关闭前强制 flush）

> 设计思想：在复杂画布编辑场景里，“频繁保存”是常态；debounce + flush，可以兼顾性能与可靠性。

---

## 7. 导入/导出 DSL：Dify 的做法与我们如何对齐

Dify 支持导入 YAML DSL，并通过后端接口转换成 draft graph：

- `examples/dify/web/app/components/workflow/update-dsl-modal.tsx`

它的关键流程是：

1. 读取 YAML
2. 基于 node 类型做“导入合法性校验”（例如某些模式不允许某些节点类型）
3. 调用后端 `importDSL` / `fetchWorkflowDraft` 获取标准化后的 `{graph:{nodes,edges,viewport}, hash, features...}`
4. 通过事件 `WORKFLOW_DATA_UPDATE` 回灌画布

这一点对 Ntx 的启示：

- Builder 的保存格式建议拆成两类：
	- **编辑态 JSON**（含 position/viewport/UI 辅助字段）
	- **运行态 DSL（scenario.yaml）**
- 导出时需要一层 `normalize + validate + compile`：
	- normalize：补齐默认值；处理临时边；修正 connected handles
	- validate：无环/可达性/必填字段/节点类型约束
	- compile：生成 `scenario.yaml` 结构

---

## 8. Ntx 侧落地建议：把 Dify 的思想映射到 `scenario.yaml`

### 8.1 我们的目标 DSL（示例）

参考：`component/conf/udp-echo-minimal/scenario.yaml`

核心结构：

- `actions.actions[]`：action 定义（id/call/with...）
- `workflows.nodes[]`：workflow 节点（id/type/action/on/edges...）

### 8.2 画布数据模型（建议）

建议我们建立自己的编辑态 JSON（类似 Dify 的 graph）：

- `nodes: ReactFlowNode<NtxNodeData>[]`
- `edges: ReactFlowEdge<NtxEdgeData>[]`
- `viewport: Viewport`

其中 `NtxNodeData` 建议至少包含：

- `ntx_node_type: 'start' | 'action' | 'wait' | 'end'`
- `title/desc`
- 若是 action 节点：
	- `action_ref: string`（引用 `actions.actions[*].id`）
	- `call: string`（可冗余存一份，导出时用于生成/校验）
	- `with: object`（最终映射到 YAML 的 `with:`）
- 若是 wait 节点：
	- `on: { event: string, match: object }`

`NtxEdgeData` 建议包含：

- `label?: string`
- `branch?: 'sent' | 'failed' | 'timeout' | string`（也可以直接用 sourceHandle 表达）

### 8.3 Node Registry：从 actions catalog 生成可拖拽节点

对齐你们“方案 1（强制 actions-executor 自描述）”：

- 后端/host 调用 actions-executor 的 `list-actions/describe-action` 生成 `actions catalog`
- 前端左侧面板展示 catalog
- 拖拽一个 action 到画布时，创建 `ntx_node_type='action'` 的节点，填入：
	- `action_ref`（默认 `${action_id}` 或 `${action_id}-${n}`）
	- `call`
	- `with`（基于 schema defaults）

### 8.4 编译导出：graph -> scenario.yaml

导出步骤建议：

1. 从 Start 节点出发计算可达节点（借鉴 `getValidTreeNodes` 思想）
2. 校验：
	 - 无环（借鉴 `getCycleEdges`）
	 - 必须存在 start/end（或你们的等价节点）
	 - action_ref 引用的 action 必须存在
	 - wait 节点必须有 `on.event`
	 - wait 的 `on.match` **允许为空**，但在常见的 packet-rx 场景需要 `action_id` 做关联（见下文“推断规则”）
3. 生成：
	 - `workflows.nodes[]`：按节点 id 输出，并从 edges 构造 `edges: [{to,label}]`
	 - `actions.actions[]`：从画布上所有 action 节点聚合（按 action_ref 去重或按节点实例输出，按你们语义决定）
	 - `actions.actions[*].with`：以 **catalog defaults 优先**（defaults → 用户覆盖）确保导出稳定

> 注意：你们的 `scenario.yaml` 里 `workflows.nodes[*].id` 与 `actions.actions[*].id` 是否允许一对多/多对一，需要在编排器里明确建模。

#### 8.4.1 关键推断规则（通用 Builder 语义）

为了保持通用 workflow builder 的可用性，同时不违背 `DESIGN_PROMPT` 中“事件驱动/单线程/不在组件内等待”的约束，我们把导出逻辑明确分为：

- **编辑态（graph）**：允许不完整（临时节点/未连线/空 match/未填参数）
- **运行态（scenario.yaml）**：导出时执行 `normalize + validate + compile`

其中 compile 存在少量“推断”，其目标是：

1) **只导出 Start 可达子图**

- 导出以 Start 节点为根做可达遍历，仅输出可达 nodes/edges。
- 目的：画布上允许保留草稿/临时分支，不污染运行态 DSL。

2) **wait.on.match.action_id 的推断（从入边 action）**

- 如果 wait 节点的 `on.match.action_id` 未显式填写：
	- 尝试从 **wait 的入边（incoming edge）** 找到上游 action 节点
	- 取上游 action 节点的 `action_ref` 作为 `action_id`
- 目的：让“action → wait(packet-rx)”这种常见模式开箱可用，并且比“从 start 推断”更通用。
- 注意：若图结构复杂（wait 有多个上游 action / 多入边），建议用户显式填写 match，避免歧义。

3) Start 导出为两种语义（兼容 demo + 保持通用）

- 编辑态里 start 始终是 `ntx_node_type='start'`。
- 导出时：
	- 若 start 的第一条出边连接到了一个 action 节点，则可将 start 导出为 `type: action`（udp-echo-minimal 风格，便于跑通最小 demo）。
	- 否则按 `type: start` 导出，保持 DSL 的通用性，不强行假设“start 必为 action”。

4) action.with 的默认值来源：**catalog 合并**（不维护 manifest）

- 前端不维护 action manifest。
- `actions-catalog.json`（由 host 侧通过 actions-executor 自描述 API 生成）是 action schema 与 defaults 的唯一真相源。
- 导出动作参数采用：

	$$with_{export} = merge(defaults_{catalog}, with_{user})$$

	其中用户在 Inspector 里编辑的 `with` 覆盖 defaults。

#### 8.4.2 场景外壳（Scenario scaffold）

`scenario.yaml` 的顶层块（`workbook/resources/load/user_resources`）本质上是“场景外壳”，不属于 workflow 图本身。

- Demo/联调用途：可以提供一个最小 scaffold（例如 udp-echo-minimal 的 udp-target + load + ip_binding）。
- 通用 builder：应支持关闭默认 scaffold 或由上层模板注入（例如“先导入已有 scenario_demo.yaml 作为模板，再只覆盖 actions/workflows”）。

---

## 11. Action Catalog（actions 列表）协议（Ntx 强制化方案）

本节定义“前端动作列表从哪来”的强制化方案：**actions-executor 自描述** + 平台生成 catalog（前端不直接跑 wasm）。

### 11.1 WIT 契约（自描述接口）

WIT 文件：`component/wit/actions-executor/world.wit`

actions-executor 组件必须导出：

- `schema-version() -> u32`
- `list-actions() -> list<action-summary>`
- `describe-action(action-id) -> result<action-spec, string>`

强约束（必须写进接入规范）：

1. `list-actions/describe-action` 必须是**纯元数据**接口：
	 - 不依赖 `event-bus` / `hostnet` / filesystem / 网络 / 时间
	 - 不做 IO、不等待、不订阅
	 - 可重复调用且返回确定性结果
2. 返回的 schema 版本用于平台缓存与兼容：
	 - `schema-version` 变化意味着 catalog 数据结构有破坏性变更

### 11.2 参数编码：统一 `with_json`（映射到 `action-def.params`）

我们统一约定 action 参数“编辑态/运行态”的传递方式：

- 前端编辑：使用 JSON 对象（UI form 由 `input-schema-json` 生成）
- 保存到 Ntx DSL（`scenario.yaml`）：仍然可以是 YAML map（便于人读）
- 编译到执行：scheduler 将 YAML map 规范化为 JSON string，填入 `ntx:core-types/types.action-def.params`

对齐现有 core-types：

- `component/wit/types/world.wit` 中：`type json = string;`
- `action-def.params: json`（即 JSON string）

因此：`with_json` 落地不需要修改 core-types，只需要在 scheduler 编译/模板展开阶段完成 YAML->JSON string。

### 11.3 平台/Host 侧 Catalog 生成与缓存

推荐流程：

1. 用户上传 `actions-executor` wasm component（或通过 WAC 组装后的 component）
2. 平台实例化 component，仅调用 `schema-version/list-actions/describe-action`
3. 生成并缓存 `actions-catalog.json`（与 component hash 绑定）
4. 前端通过 API 获取 catalog，并渲染动作面板

缓存键建议：

- `component_sha256 + schema_version`

### 11.4 Catalog 到画布节点的映射

当用户从动作面板拖拽一个 action 到画布：

- 创建一个 workflow 节点（`ntx_node_type='action'`）
- 设置：
	- `action_ref`（节点实例 id，可重复，如 `udp-send-reply#1`）
	- `call`（来自 catalog）
	- `with`（来自 `defaults-json`，编辑态为 JSON 对象）

导出到 `scenario.yaml` 时：

- 节点引用到 actions：
	- `workflows.nodes[*].action = <actions.actions[*].id>`

动作定义：

- `actions.actions[*].id`：取 `action_ref`（或按你们语义决定是否一对多共享）
- `actions.actions[*].call`：取 catalog 的 `call`
- `actions.actions[*].with`：取节点当前参数（最终 YAML map）

> 兼容性提示：当前仓库 generator（`actions-catalog-gen`）输出为 kebab-case 字段名（如 `schema-version`、`default-params-json`）。
> 前端解析时应兼容 snake_case 与 kebab-case，避免目录间不同版本 catalog 造成 defaults 丢失。

---

## 12. 校验策略（Validation）——对齐 DESIGN_PROMPT 的运行限制

校验分两层：

1) **编辑态提示（non-blocking warnings）**：允许用户暂存不完整图
2) **导出前校验（blocking errors）**：确保运行态 DSL 满足 scheduler 的事件驱动执行模型

建议的最小规则集（并已在 demo 中落地一部分）：

- Start：
	- 无 start：warning
	- 多 start：warning（导出只取其中一个做可达根）
	- start 无出边：warning
	- start 多出边：warning（导出可能把第一条当主路径，建议显式分支语义）
- Action：
	- 缺 action_ref/call：error
	- call 不在 catalog：warning（可能运行时 unknown action）
- Wait：
	- 缺 on.event：error
	- 缺 on.match.action_id：warning（导出会尝试从入边推断，建议显式填写）
- End：
	- end 有出边：warning
- Export：
	- 不可达节点：warning（导出只包含 start 可达子图）

---

## 13. 场景外壳（Scenario Scaffold）模板注入（通用 Builder 必备）

> 背景：`scenario.yaml` 顶层的 `workbook/resources/load/user_resources` 等属于“场景外壳”，它们不是 workflow 图本身。
> 在 demo 里我们可以写死一个 udp-target 作为便捷 scaffold，但通用 workflow builder 必须支持“模板注入”，否则导出会长期处于“缺很多字段/不可运行”。

### 13.1 导出契约（推荐不变）

将导出明确拆成两部分输入：

- **Graph（编辑器产物）**：`nodes/edges` 与 actions catalog
- **ScaffoldTemplate（外壳模板）**：一个已有的 scenario 对象（来自粘贴/导入/平台预设）

输出：

- **Scenario（运行态 DSL）**：最终写入 `scenario.yaml`

契约形式：

$$Scenario = merge(ScaffoldTemplate, Compile(Graph, Catalog))$$

其中：

- `Compile(Graph, Catalog)` 只负责生成/覆盖：
	- `actions.actions[]`
	- `workflows.nodes[]`
	- （可选）`name` / `version`（按产品策略决定是否允许用户覆盖）
- `ScaffoldTemplate` 提供：
	- `workbook`（resources/ip_pools 等）
	- `load`（ramp_up/user_lifetime 等）
	- `user_resources`（ip_binding 等）
	- 以及未来扩展字段

> 这样做的好处：
> - workflow builder 不需要知道所有资源/负载字段的细节（保持通用与轻量）
> - 用户可以复用已有 demo、或从平台选择标准模板
> - 完全符合 `DESIGN_PROMPT`：纯声明式合成，不引入运行时等待/重入/动态入口

### 13.2 两种模式（便于落地与演进）

- **模式 A：Demo 便捷模式（内置 scaffold）**
	- 仅用于 demo-workflow/快速联调。
	- exporter 提供一个最小 udp-echo scaffold（例如 udp-target + load + ip_binding）。

- **模式 B：通用模板注入模式（推荐默认）**
	- 用户导入一个现有 scenario（YAML 或 JSON）作为 `ScaffoldTemplate`。
	- exporter 合并后只替换 actions/workflows。
	- 典型输入：`component/conf/udp-echo-minimal/scenario_demo.yaml`。

### 13.3 模板注入的最小校验点（建议）

模板注入后，builder 可以做一些“软校验”，避免导出后运行时报错：

1) **资源引用检查（warning）**
	 - 若 `action.with` 中出现 `target: "udp-target"` 这类值，建议检查该 id 是否存在于 `workbook.resources[*].id`。
	 - 注意：这属于启发式检查（action 参数是开放 JSON），不应强行阻塞导出。

2) **load / user_resources 存在性提示（warning）**
	 - 若模板缺少 `load` 或关键字段，提示“可能不会 spawn user / 不会触发执行”（具体取决于 scheduler 默认行为）。

3) **catalog 一致性提示（warning）**
	 - 模板与 catalog 可能来自不同版本的 executor。
	 - 若 `actions.actions[*].call` 不在 catalog，提示 unknown action 风险。

### 13.4 demo-workflow 的参考实现（建议）

在 demo-workflow 中提供一个简单 UI：

- 一个 textarea：允许粘贴 YAML 或 JSON 作为 ScaffoldTemplate
- 一个开关：是否启用模板注入（启用时关闭内置 demo scaffold）
- 导出时：
	- parse（YAML/JSON）→ object
	- merge + compile
	- 将最终 scenario 以 YAML 文本展示/复制

> 注意：为减少依赖，demo 可以只支持 JSON（或 YAML→JSON 的极简解析）；正式产品建议用成熟 YAML parser。


---

## 9. 推荐模块结构（Ntx 前端工程落地参考）

建议我们在 Ntx 前端项目中沿用 Dify 的“分层 + util 抽象”的方式：

- `graph/`
	- `canvas/`（ReactFlow 包装）
	- `nodes/`（Action/Wait/Start/End/Note 的渲染壳）
	- `edges/`（自定义 edge + label/插入节点）
	- `store/`（zustand slices：graph-slice、ui-slice、draft-slice、panel-slice）
	- `registry/`（actions catalog -> node factory/schema/form renderer）
	- `compile/`（graph -> scenario.yaml）
	- `validate/`（cycle、可达性、必填字段、端口匹配）
	- `layout/`（ELK 自动布局）

---

## 10. 后续工作（建议按里程碑）

1. M1：最小可用编排器
	 - start/action/wait/end + 连线 + 节点编辑
	 - 保存/恢复编辑态 JSON
	 - 导出 `scenario.yaml`
2. M2：高级体验
	 - edge 上插入节点（参考 Dify custom-edge）
	 - 自动布局（ELK）
	 - 校验错误高亮（cycle edges/invalid nodes）
3. M3：与 actions-executor catalog 对接
	 - 左侧 action 列表来自 `list-actions/describe-action`
	 - 参数面板按 schema 自动生成

