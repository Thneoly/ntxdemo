调度器（scheduler）与执行器（actions-executor）设计说明
===========================================================

> 目标：设计一套 **wasm32-wasip2 组件形态** 的调度器和执行器，使用 **workflow + workbook** 驱动的事件增强型状态机，实现多用户、多任务、可编排、可观测的负载场景执行系统。

> 关键约束（更新）：本设计最终以 **WAC 组装后的单一 composed component** 交由 **host（wasmtime runtime）** 加载运行。
> 因 wasm32-wasip2 组件内天然单线程，本方案中的 “async” 指的是：
> - **WIT 层面可 `await` 的异步 ABI**（`async func` / `stream` / `pollable`），使 host 能用 Tokio/async runtime 以“可取消/可超时/可背压”的方式驱动组件；
> - guest 组件内部仍保持单线程，但允许通过 async ABI 表达“等待事件/等待 IO”而不阻塞 host；
> - 真正的并行（多 executor 实例、NIC 收包、计时器驱动等）由 host 侧调度实现。

补充说明（重要）：目前仓库中的 `component/wit/**/*.wit` 采用的是 **同步导出 + poll/wait 风格**（例如 `eventbus.wait-events(timeout-ms)`、`scheduler-component.run()`），可视为 **v0.1 过渡 ABI**。

本文档后续章节将给出一个 **WIT Async ABI vNext**：在保持组件内部单线程的前提下，把对 host 的导出/导入接口升级为 `async func` / `stream` / `pollable`，从而让 host（Tokio）不再依赖“额外线程跑 run()”也能驱动 guest 事件循环。

---

一、整体运行形态与职责
----------------------

### 0. 组件组装与运行时（WAC + wasmtime）

最终交付物不是三个 wasm 单体，而是一个由 **WAC** 组装的 `scheduler-composed.wasm`：

- **eventbus component**：导出 `ntx:scenario-eventbus/event-bus@0.1.0`
- **actions-executor component**：导入 eventbus，导出 `ntx:scenario-actions-executor/action-component@0.1.0`
- **scheduler component**：导入 eventbus + actions-executor，并导出：
  - `ntx:scenario-scheduler/scheduler-component@0.1.0#run(config-dir)`
  - `ntx:scenario-scheduler/packet-ingest@0.1.0#notify-rx(desc-mem, payload-mem)`

host（wasmtime）只需要加载 **组装后的 composed component**，并为其补齐 “host world” 导入（如 udp-socket-control、资源访问等）。

> 设计目的：把 eventbus/scheduler/actions-executor 的依赖在组件内部用 WAC 固定，host 侧只对接 hostnet / NIC / timer / config / observability。

### 1. scheduler 运行形态

- **运行形态**：`wasm32-wasip2` component  
- **核心职责（逻辑主循环）**：
  - 从 `eventbus` 接收事件（外部事件、action 执行结果、定时器事件、控制事件等）
  - 基于 **workflow+workbook 生成的状态机** 更新用户/任务状态
  - 挑选可运行的 task，调度给 `actions-executor` 执行
  - 接收 `actions-executor` 回传的结果事件，再次驱动状态机前进
  - 将关键状态变化和统计信息回写到 `eventbus` 或其他观测通道

**抽象主循环**（伪代码，仅说明行为）：

```text
loop {
  // 1. 从 eventbus / host 拉取一批事件（阻塞或带超时）
  // async 模式：优先使用 wait-events(subscription, max, timeout)
  // - timeout>0：允许在无事件时阻塞等待，避免忙轮询
  // - timeout=0：等价 poll-events（非阻塞）
  events = receive_events_via_eventbus_wait()

    // 2. 应用事件到内部状态机
    for e in events {
        apply_event_to_state_machine(e)
    }

    // 3. 计算新的就绪 task（考虑优先级、配额、并发度）
    ready_tasks = scheduler_select_ready_tasks()

  // 4. 将就绪 task 派发给 actions-executor
  // async 模式：scheduler 不等待“真实网络 IO 完成”，只产生事件并推进状态。
  // - 对需要网络收包的步骤：task 进入 Waiting，等待 packet.rx 或 timer 事件
  // - 对需要发包：actions-executor/scheduler 发布 packet.tx-request 事件，host 按事件驱动发包
  dispatch_to_actions_executor(ready_tasks)

    // 5. 生成/更新指标、心跳等
    emit_metrics_and_heartbeat()
}

#### 1.0 Host 如何拉起 scheduler 的主循环（WAC 组装后的启动契约）

本仓库中，scheduler 作为 `wasm32-wasip2` component 被 **WAC 组装**（见 `component/wac/scheduler-composition.wac`）后，会对外导出两个接口实例：

- `ntx:scenario-scheduler/scheduler-component@0.1.0`
- `ntx:scenario-scheduler/packet-ingest@0.1.0`

其中，`scheduler-component` 明确提供 scheduler 的主循环入口（见 `component/wit/scheduler/world.wit`）：

- `run(config-dir: string) -> result<_, string>`

**Host 启动流程必须先调用 `run()` 拉起 scheduler 的调度循环**，之后才会进入后续的收包驱动（`notify-rx`）与发包调度（事件总线）链路。

（更新：async 模式的 run 语义）

- `run()` 仍然是一个长期运行的“事件循环入口”。
- run 内部不会创建线程，但会通过 eventbus 的 `wait-events(.., timeout_ms)` 进行 **可等待的事件获取**，从而具备 async 语义（无事件时让出 CPU）。
- host 侧可选择：
  - **单独线程**运行 `run()`（推荐，隔离长期运行与其他 host 任务）
  - 或将 `run()` 做成一个 “组件专用线程/任务”，与 NIC RX/控制面并行

> 重要：为了支持“WIT 层面的 async（async func/stream/pollable）”，本节描述的 `run()` 仅代表 **当前 v0.1 同步 ABI 的行为**。
> 当启用 WIT Async ABI vNext 后，应以 **可轮询推进（tick/poll）或事件流（stream）** 替代“长期阻塞的 run()”，避免 host 被迫用线程隔离。

推荐的 host 行为模型如下：

1. host 加载 `component/wac/scheduler-composed.wasm`（WAC 产物），并在启动时配置 `scheduler.wasm.component_path` 指向该文件。
2. host 启动后会先初始化 **host scheduler**（root crate 的 `Scheduler`），并由该 host scheduler 统一调度：
  - NIC RX resident 任务（持续收包）
  - wasm 相关调用（WasmCall 任务）
3. （**按当前实现**）host 会向 host scheduler 提交一个 “run” 类型的 wasm 调用任务，用于触发 guest 的 `run()`：
  - 入口参考：`src/main.rs` 中提交 `Task::wasm_call("wasm-run", "run")`，随后 `Scheduler::global().run()` 进入 host 调度循环。
  - wasm engine 的加载参考：`src/scheduler.rs::apply_wasm_config()` 会通过 `EngineManager::load_and_register()` 实例化 composed component。
4. （**接口契约**）当 host 需要真正拉起 component scheduler 的主循环时，应该由 wasm engine 调用导出
  `scheduler-component.run(config-dir)`；其中 `config-dir` 指向 scenario/workflow/workbook/load 的所在目录。
5. 为避免阻塞 host 的其他子系统（例如 NIC RX poll、控制面、日志等），host **可以在单独线程**中执行该 `run()`：
  - 该线程负责长期运行 scheduler 的 loop；
  - 其他线程（例如 NIC RX 线程）通过 host→guest 的导出接口调用（例如 `packet-ingest.notify-rx`）向 scheduler 注入外部刺激。

> 备注：`run()` 是“拉起组件内部事件循环”的入口；`notify-rx()` 是“外部事件注入”的入口。两者不是互斥关系，而是 **先 run，再 notify**。
>
> 现状对齐：当前 host 的 wasm engine 已稳定支持 `packet-ingest.notify-rx(desc_mem, payload_mem)`（用于收包驱动）；
> `scheduler-component.run(config-dir)` 已在 WIT/WAC 中定义并导出，但 host 侧还需要在 wasm engine 的 WasmCall 执行路径中把 `run()` 真正调起来（目前 `TaskKind::WasmCall` 分支只做日志占位）。

#### 1.1 Host 侧当前实现（与仓库 `src/` 对齐）

本仓库的 host 采用 **wasmtime component model** 加载 WAC 组装产物，并通过一个 **host scheduler（root crate 的 `Scheduler`）** 负责：

- NIC RX 轮询（resident task，非阻塞）
- 定时器任务（TimerTask，触发后写入 host event-bus）
- wasm engine 调用（目前保留为扩展点）

对应代码入口：

- `src/main.rs`
  - 主线程运行 `scheduler::Scheduler::global().run()`（阻塞保持进程存活）
  - 额外启动一个线程 `ntx-guest-scheduler` 专门调用 guest 的 `scheduler-component.run(config-dir)`（该调用预期长期阻塞）
- `src/scheduler.rs`
  - `NetworkIoTask::NicRx` 作为 resident task 注册（默认 `netio-wait`）
  - NicRx 在 host 侧通过 `kernel::non_blocking_recv_udp()` 取包（无包立即返回）
  - 取到包后，使用 `src/wasm_engine/shared_mem.rs` 的布局将其写入两段 buffer：
    - `desc_mem`：control + desc ring
    - `payload_mem`：payload 字节区
  - 然后直接调用 `EngineManager::notify_rx(desc_mem, payload_mem)` 进入 guest 的 `packet-ingest.notify-rx(..)`
  - 注：当前 `TaskKind::WasmCall` 仍是占位（仅日志），NicRx 已内联执行 notify-rx，因此 WasmCall 暂不承担 packet-ingest 调用。
- `src/wasm_engine/engine.rs`
  - 明确按 WIT 合约绑定并调用：
    - `ntx:scenario-scheduler/packet-ingest@0.1.0#notify-rx(desc_mem, payload_mem) -> result<u32, string>`
    - `ntx:scenario-scheduler/scheduler-component@0.1.0#run(config-dir) -> result<_, string>`
- `src/wasm_engine/manager.rs`
  - `EngineManager` 维护多个 `ComponentEngine`（当前以 default 为主），提供 `notify_rx(..)` 与 `run(..)` 的路由

这一实现与本文档的 async 口径一致：

- guest scheduler 内部通过 eventbus `wait-events(timeout)` 低 CPU 等待事件
- host 侧并行运行：
  - “长期阻塞”的 guest `run()`（独立线程）
  - “短耗时、高频”的 NIC RX → notify-rx（host scheduler loop）

#### 1.2 WIT Async ABI vNext（async func / stream / pollable）

本节定义“把 component 改为 async 模式”的目标形态：

- 对 host：导出接口是 **可 `await` 的**，并支持 **取消/超时**（由 host runtime 的 cancellation/timeout 语义驱动）。
- 对 guest：内部仍是单线程事件循环，但“等待事件/等待 IO”通过 `pollable` 或 `stream` 交还 host，而不是在一次导出调用里长期阻塞。

vNext 方案选择：**vNext-A（tick + pollable）**。

原因：它与现有“单线程事件循环 + wait/poll”模型最接近，最易做预算控制、背压与可观测性，同时让 host（Tokio）获得“可 await/可取消/可超时”的驱动能力。

##### 1.2.1 vNext-A 的契约（行为）

- `init()`：完成加载配置、订阅 eventbus、初始化状态机与资源映射。
- `ready()`：返回一个 `pollable`，用于表示“现在值得调用一次 tick”（例如：有新事件、有定时器到期、内部队列非空、或有待处理的 RX 注入）。
- `tick(budget-ms)`：在预算内推进事件循环与调度（可包含处理若干批事件、派发若干 task），返回本次处理量（建议返回 `u32` 事件数或工作量计数）。
- （可选）`shutdown()`：显式退出/释放资源，让 host 能优雅停止场景。

- `notify-rx(desc-mem, payload-mem)`（async）：用于 host 将“RX ring 的一批新增数据”注入 guest。
  - guest **不得**在 `notify-rx` 内做长时间不可取消等待；职责是把数据解析为内部队列/事件（例如生成 `packet.rx` 或写入 pending-rx 队列）。
  - `notify-rx` 成功写入 pending 队列后，应使 `ready()` 尽快就绪（pollable 变为可读），提示 host 调用 `tick()` 消化。

> 重要：vNext-A 的关键点是 **组件导出永远不做“不可取消的长期阻塞”**。等待由 `ready(): pollable` 交还 host 统一 poll。

##### 1.2.2 vNext-A 的 WIT 签名草案（建议）

以下为建议草案（用于文档对齐；最终签名以你们落地时 WIT/绑定生成能力为准）：

- `scheduler-component@vnext`（建议通过新 package 版本或新 interface 名称实现隔离）
  - `init: async func(config-dir: string) -> result<_, string>`
  - `ready: func() -> pollable`
  - `tick: async func(budget-ms: u32) -> result<u32, string>`
  - `shutdown: async func() -> result<_, string>`（可选）

- `packet-ingest@vnext`
  - `notify-rx: async func(desc-mem: list<u8>, payload-mem: list<u8>) -> result<u32, string>`

  说明：把 `notify-rx` 升级为 `async func` 的主要目的不是“在 guest 内 await 网络”，而是让 host 能把它纳入统一的 **超时/取消/背压** 控制；同时避免在 host 侧产生额外的阻塞线程。

##### 1.2.3 host（Tokio）推荐驱动循环（vNext-A）

推荐的 host 驱动方式是“单实例串行调用 + poll ready + tick 预算推进”：

- host 创建一个 **Wasm Engine Actor**（单任务持有 store/instance），对外提供异步请求队列。
- driver 任务逻辑：
  1. `await init(config_dir)`
  2. 循环：
     - `p = ready()`
     - `await p`（或将其注册到 host poll/selector）
     - `await tick(budget_ms)`
  3. 收到停止信号时：`await shutdown()`（若提供）

背压建议：

- NIC RX → `notify-rx` 走有界队列（聚合/丢弃策略由 host 决定，例如按 flow/user 聚合），避免无限堆积。
- `tick(budget_ms)` 的预算应该可配置（例如 1~5ms），以平衡延迟与吞吐。

##### 1.2.4 notify-rx 的超时/取消/背压治理（host 统一策略）

由于 NIC RX 是高频、突发且可能持续过载的输入源，vNext-A 要求 host 对 `notify-rx` 的调用实行统一治理。

- **超时（timeout）**
  - host 对每次 `notify-rx` 设置超时（建议 1~5ms，按实际负载调参）。
  - 超时后 host 应取消该次调用，并按背压策略处理（见下）。

- **取消（cancellation）**
  - 当场景停止、实例重启或 host 需要快速回收资源时，应能取消正在进行的 `notify-rx` / `tick`。
  - 取消只保证“host 侧不再等待结果”；guest 侧要求：被取消的调用不得导致内部状态损坏（允许丢弃本批 RX 或仅处理部分条目）。

- **背压（backpressure）**
  - host 必须使用**有界队列**承接 RX→wasm 注入（而不是直接无界堆积 `desc_mem/payload_mem`）。
  - 队列满时推荐优先策略：
    1) **merge/coalesce**：对同 flow/user 的多个 RX 合并为更大的一批（减少 wasm call 次数）。
    2) **drop-new**：丢弃最新的 RX 批次，并记录 `rx_dropped` 指标。
    3) （可选）**drop-old**：丢弃最旧批次，以降低尾延迟（适合“最新数据优先”的场景）。

为便于跨语言绑定与快速落地，建议在 `result<_, string>` 的 error string 中形成可机器识别前缀（示例）：

- `"cancelled"`：调用在 host 侧被取消或 guest 检测到取消。
- `"timeout"`：超时（host 侧或 guest 侧软超时）。
- `"overloaded"`：guest 内部队列已满（guest 侧二级背压）。

> 注：后续若你们愿意升级错误类型，建议把 `string` 替换为结构化 error（例如 `record { code, message }`），但这属于 WIT 进一步细化，不影响 vNext-A 主线。

#### 1.3 兼容与迁移策略（v0.1 → vNext）

为避免一次性改动过大，建议按以下策略迁移：

1. 保留 v0.1（同步 ABI）一段时间作为兼容层：`run()` / `notify-rx()` / `wait-events()` 继续可用。
2. 新增 vNext async 接口（新 package 版本或新 interface 名称），host 侧先以 feature flag 切换驱动方式。
3. 当 vNext 稳定后，再逐步废弃 `run()` 的“长期阻塞”语义。

#### 1.4 受影响文件与后续改动清单（落实 vNext 必需）

当你们决定“把 component 改为 WIT 层面的 async（async func / stream / pollable）”后，除了本文档更新外，代码层面至少会影响以下位置：

- WIT（接口契约）
  - `component/wit/scheduler/world.wit`
    - 需要新增/升级 `scheduler-component` 的 vNext-A 接口（`init/tick/ready(pollable)`），并确定版本策略（新 package 版本或新 interface 名称）。
    - `packet-ingest.notify-rx` 在 vNext-A 下建议升级为 `async func`，以便 host 统一实施超时/取消/背压治理。
  - `component/wit/eventbus/world.wit`
    - vNext-A 并不强制修改 eventbus 的 schema，但可按需要补充“就绪信号”（pollable）以降低 tick 空转。
    - 事件类型（record/event schema）通常可保持不变。

- WAC（组装）
  - `component/wac/scheduler-composition.wac`
    - 导出世界与实例名可能发生变化（例如新增 vNext interface 导出），需要同步更新导出项。

- Host（wasmtime 调用模型）
  - `src/wasm_engine/engine.rs` / `src/wasm_engine/manager.rs`
    - 需要从“同步 typed call”迁移到“Tokio async 驱动 + 轮询 pollable + tick/notify-rx”的调用方式。
    - 需要明确 Store/实例的并发访问策略：推荐用单一任务/actor 持有 store，避免并发进入同一实例。
  - `src/scheduler.rs`
    - 当前 `TaskKind::WasmCall` 是占位；在 vNext 下它应演进为“异步 wasm 调用执行队列”，承载 tick/notify-rx 等调用。

> 备注：本文档只定义 vNext-A 的目标 ABI 与运行语义；具体 WIT vNext 的最终签名请以你们落地时的 WIT 版本与 wasmtime 绑定生成能力为准。

（旧的“1.2 面向 Tokio/异步 host 的组件 ABI 建议”已合并进本节的 vNext 定义；v0.1 同步 ABI 仍可保留作为过渡实现。）
```

### 2. actions-executor 运行形态

- **运行形态**：`wasm32-wasip2` component  
- **角色定位**：
  - 作为 **task 中一个或多个 action 的具体执行单元**
  - 为 scheduler 提供一组 **同步或异步的 action 执行接口**
  - 封装协议细节（如 HTTP/TCP/UDP 调用、socket 绑定 IP、数据格式转换等）
- **与 scheduler 的协作方式**：
  - scheduler 通过 WIT 接口调用 `execute_action/execute_task` 等方法
  - （async 模式）actions-executor **不阻塞等待外部 IO**：
    - 若 action 可以在本次调用内完成，则直接返回 outcome
    - 若 action 需要 host 侧异步能力（发包、等待回包、异步定时等），executor 将通过 eventbus 发布请求类事件（如 `packet.tx-request` / `send.schedule-request`），并由 scheduler 进入 Waiting，后续通过 `packet.rx` / `scheduler.timer.*` / `scheduler.action-result` 等事件闭环推进

### 3. 多用户/多 task 视角

- **user**：负载模型中的基本主体，一个 user 通常对应一条 workflow 实例链路
  - 一个 user 可以有多个 task
  - task 内包含多个 action
- **task**：workflow 中的一个节点（node），也是一次 actions-executor 的执行单元（或多个 action 的组合）
- **action**：更细粒度的执行动作，例如一次 HTTP 请求、一次 socket 建连/收发等
- **event**：驱动状态变化的触发器，包括：
  - 外部事件（如系统控制命令、资源就绪事件）
  - 内部事件（如 action 执行结果）
  - 定时器事件（如超时、心跳、think-time 结束）

---

二、设计约束与原则（融合 PMP 映射与事件唯一入口）
-------------------------------------------

### 1. PMP → 本系统的映射关系

- **Project** → 一个 scheduler 实例（或一个独立的场景运行）
- **Workflow / WBS** → workflow 图；其中每个 node = 一个 task
- **Work Package** → task（可被调度的工作单元）
- **Activity** → action（task 内最小执行语义）
- **Deliverable** → action result（通过 event 暴露出来的可观测结果）
- **Workbook** → 运行态的快照：state + metrics + history 的综合视图
- **Change Request** → 能修改 workflow / workbook 的事件（TopologyControlEvent 等）

> 换句话说：scheduler 更像是「项目运行引擎」，而不是单纯的「执行引擎」。

### 2. wasm32-wasip2 单线程与调度模型

1. **单线程约束**
   - scheduler / actions-executor 组件内部只能单线程执行
  - **禁止** 在组件内部创建线程或进行不可中断的长时间阻塞 IO
  - （async 更新）允许使用 eventbus 的 `wait-events(timeout)` 进行 **受控阻塞等待**，用于避免忙轮询。
    - 说明：这里的 “阻塞” 是协作式等待（超时轮询 + 短 sleep/yield），并不依赖 OS 线程 park/condvar。
2. **调度与并行的正确来源**
   - scheduler 本身是一个 **单线程事件循环**，只做：
     - 决策（基于状态机与优先级）
     - 派发（调用 actions-executor）
     - 收集结果（转化为 event）
   - 并行来自：
     - host 层同时实例化 **多个 actions-executor component 实例**
     - 或 host 层并行运行多个 scheduler 实例（多 Project）
   - scheduler 不需要知道具体并行细节，只需要把 task 派发到「某个可用的 executor 实例」即可。

### 3. 事件驱动优先（事件是唯一合法的“动态入口”）

- 所有「动态变化」（包括：
  - task 状态变化
  - user 生命周期变化
  - 资源分配/释放
  - workflow 拓扑调整
  ）都必须通过 **Event** 体现
- **禁止** 在没有事件的情况下，直接修改内部状态：
  - 不允许从外部直接写入 scheduler 内部结构
  - 所有控制类动作（暂停/恢复/变更配置）均通过控制事件完成
- 这样可以保证：
  - 可回放：重放同一序列事件应得到相同的状态演化
  - 可审计：任何状态变化都有事件可追踪

### 4. 可插拔、可扩展

- workflow / workbook / action 定义使用 **声明式 YAML/JSON**，不写死在代码中
- 允许新增：
  - action 类型（如 `http.get` / `sip.send` / `custom.xxx`）
  - 资源类型
  - 触发条件表达式
- scheduler 内部只依赖抽象模型（ActionDef / ResourceDef / TriggerDef），不依赖具体协议实现。

### 5. 可观测性与运行状态

- 关键事件（user 生命周期、task 转移、action 结果、资源分配/释放）均可通过 eventbus 对外暴露
- 需要有基本的统计：QPS / 失败率 / 延迟分布（粗粒度）等
- scheduler 自身也有一个「运行状态」：
  - `Idle` / `Running` / `Degraded` / `Error` / `Completed`
  - 其变化同样通过事件对外暴露（如 `scheduler.state-changed`）

---

三、状态机模型（workflow + workbook 驱动）
----------------------------------------

### 1. 状态 / 静态 / 动态的划分

- **状态（State）**：  
  - 描述系统从一个「全局状态」演进到另一个「全局状态」的过程
  - 包含所有用户、task、action 的当前执行位置、资源占用等信息

- **静态（Static Topology）**：  
  - 某一时刻，workflow 中的 **节点（node）**、**边（edge）**、**触发条件（trigger）** 的集合
  - 关注的是「是否存在」：某个 node / edge / trigger 是否存在、结构是否满足约束

- **动态（Dynamic Evolution）**：  
  - 在连续两个时刻之间，静态拓扑及其附带元数据（优先级、权重、绑定资源等）的变化
  - 例如：新增/删除 node、修改 edge 的触发条件、切换某些节点的优先级

### 2. 状态机中的核心对象（校准版）

- **user（资源与生命周期的基本单位）**
  - workflow 的「实例级」主体，拥有独立的生命周期
  - **重点**：user 不是 task 的简单容器，而是「资源边界」：
    - 所有与该 user 相关的资源（IP、端口、session、cookie、fd 等）都挂在 user 上
    - task 在执行时通过 user 的 context 访问/占用这些资源
  - 属性示例：
    - `user_id`、`tenant_id`
    - `state`（UserRunning / UserCompleted / UserFailed …）
    - `current_node`（当前所在 workflow 节点）
    - `resources`（IP 绑定、会话信息等）
    - `metrics`（累计次数、失败率等）

- **task（调度与状态机的核心节点）**
  - workflow 中的节点，也是 scheduler 的最小调度单位
  - 对 actions-executor 来说，一个 task 通常映射为「一次可执行单元」（可以包含一个或多个 action）
  - 关键属性：
    - `task_id`
    - `workflow_node_ref`（指向静态 workflow 定义）
    - `state`：`Pending / Runnable / Running / Blocked / Done / Failed / Cancelled`
    - `priority`：调度优先级
    - `actions[]`：task 内包含的 action 列表或引用
    - `triggers`：出边触发条件集合
    - `context`：本 task 局部变量（如模板展开后的 URL、重试计数等）

- **action（最小执行语义）**
  - 具体执行动作，例如：
    - HTTP 请求（method、url、headers、body 模板）
    - TCP 连接 + 若干读写
    - 自定义协议 / 插件
  - 关键属性：
    - `type`（如 `http.get`、`tcp.send`、`custom.xxx`）
    - `params`（WIT/JSON/yaml 结构，用于描述请求细节）
    - `timeout`、`retry` 策略
  - 可从 **actions 配置** 中解析得到。

- **event（唯一“变化源”）**
  - 触发状态变化的输入，是动态演化的唯一合法入口
  - 关键信息：
    - `source`：`action / timer / external / internal / topology-change`
    - `type`：如 `action-result`、`timer-fired`、`user-start`、`scheduler-state-changed`
    - `user_id` / `task_id` / `action_id`
    - `payload`：承载结果、错误、拓扑变更描述等
    - `timestamp`、`correlation_id`
  - 定时器也是 event 的一种，通常带有 `timer_id`、`deadline` 等字段

### 3. task 生命周期（状态机层面）

建议的 task 实例状态（每个 user 在某个 workflow 中都会生成多个 task 实例）：

- `Created`：根据 workflow 拓扑，为 user 实例化 task，但尚未调度执行
- `Ready`：满足进入条件（前置 task 完成、资源就绪、时间到等），可被 scheduler 选中执行
- `Running`：已下发到 actions-executor，等待执行结果
- `Waiting`：等待外部事件（如异步回调、定时器）完成
- `Completed`：正常执行完成，触发后继节点的就绪检查
- `Failed`：执行失败，等待重试策略或错误处理策略决定下一步
- `Cancelled`：被上层控制逻辑主动取消

状态转换完全由事件驱动，例如：

- `ActionResult(success=true)` → `Running` → `Completed`
- `ActionResult(success=false)` → `Running` → `Failed`
- `TimerFired(retry_deadline)` → `Failed` → `Ready`（重试）

#### 3.1 task 内部 step（对齐当前实现）

在更贴近工程实现的模型里，一个 workflow 的 `type: action` 节点并不一定只有一个 action，而是可以包含一个 **step 列表**（每个 step 绑定一个 action，并可覆写 timeout/retry，以及定义失败/超时的 step 跳转）。

关键约束（重要）：
- **step index 属于状态机内部字段**（per-user + per-node），不挂在 `vars` 上，避免被模板变量覆盖/污染。
- 只有当一个 action-node 的 **最后一个 step 成功** 时，才会沿 workflow edge 推进到下一个节点。
- 对于失败/超时：
  - 若 step 仍有重试次数，则由 retry timer 驱动回到本 step 继续尝试
  - 若无重试且配置了 `on_failed_step` / `on_timeout_step`，则在 action-node 内部跳转到指定 step 并继续执行
  - 否则才进入 workflow edge 的 failed/timeout 分支推进

在引入网络事件后，`packet.rx` 也会作为状态机的输入之一，例如 UDP Echo Client 场景：

```text
// 假设某个 workflow 节点 N 代表 “发送 UDP 请求并等待 Echo 回复”

// 1. 初始：task 处于 Ready，被 scheduler 派发执行
Event: SchedulerDispatch(user_id, task_id, action_id="udp-echo-client")
State: Ready → Running

// 2. actions-executor 执行 udp.send-reply，构造 packet.tx-request 并由 scheduler/host 发包，
//    然后将该 task 标记为 Waiting（等待网络回复）
Event: ActionResult(success=true, call="udp.send-reply")
State: Running → Waiting

// 3. host 收到 echo 回复，写入 RX ring，scheduler 通过 packet-ingest.notify-rx 解析出数据包，
//    按 sock_ctx 映射到对应 user/task/action，生成 PacketRx 事件
Event: PacketRx(user_id, task_id, action_id="udp-echo-client", payload=...)
State: Waiting → Completed （若 payload 校验通过）

// 4. 若 payload 校验失败或超时，则可进入 Failed/Retry 等分支
Event: PacketRx(payload_mismatch) → Waiting → Failed
Event: TimerFired(retry_deadline) → Failed → Ready （重试）
```

通过这种方式，可以把「纯网络字节流」转化为「状态机上的显式事件」，保证：

- 收包与 task lifecycle 紧密耦合（哪一个 user 的哪一个 task 收到了哪一条包）
- 所有网络行为都可通过事件重放来还原状态机演化过程。

### 4. 动态拓扑修改

调度器需要支持 **在运行时动态修改状态机拓扑**：

- 新增 / 删除 task 节点
- 修改边的触发条件、权重、优先级
- 替换某个节点关联的 action 集合

触发方式：

- 由 actions-executor 在执行过程中，根据业务逻辑发出「控制事件」（如：`AddNode`、`RemoveNode`、`UpdateEdge`）
- 或由外部控制平面通过 eventbus 向 scheduler 发送「管理事件」

scheduler 接收到这些事件后，在 **静态拓扑层** 更新 workflow 视图，再投影到后续新创建的 user / task 实例上；必要时对已有实例进行迁移或终止。

---

四、事件模型与 eventbus 交互
----------------------------

### 1. 事件分类

- **外部事件（ExternalEvent）**
  - 来自 host / 业务系统，如：开始/停止某个场景、变更负载参数、强制终止用户等
- **action 结果事件（ActionResultEvent）**
  - 来自 actions-executor，包含：
    - `status`（success/failed/timeout/partial）
    - `latency`、`response_code`、简要 `detail` 等
- **定时器事件（TimerEvent）**
  - 用于实现：
    - 用户 think-time
    - 超时控制
    - 周期性心跳/统计
- **拓扑控制事件（TopologyControlEvent）**
  - 用于动态修改 workflow / workbook

### 2. 事件基础结构（抽象）

不绑定具体语言，只描述字段：

- `id`: 事件唯一标识
- `type`: 事件类型字符串，如 `"action-result" | "timer-fired" | "user-start" | ..."`
- `user_id`: 可选，归属的 user
- `task_id`: 可选，归属的 task
- `action_id`: 可选，归属的 action
- `payload`: 任意结构（map/json/yaml），承载业务/控制信息
- `timestamp`: 事件发生时间
- `correlation_id`: 关联一次调用链路的 ID（方便追踪）

### 3. 与 eventbus 的交互模式（WIT 粗粒度约束）

调度器需要通过 WIT 接口与 eventbus 进行交互，约束如下（仅示意接口形态，不限制实现细节）：

- `eventbus.subscribe(topic_filter: string) -> result<string, string>`

- `eventbus.subscribe(topic_filter: string) -> result<string, string>`
  - scheduler 在初始化时订阅其关注的事件类别，例如：
    - 调度控制事件：`"scheduler.control.*"`
    - action 结果事件：`"scheduler.action-result"`
    - 发包请求事件：`"packet.tx-request"`
    - 收包事件：`"packet.rx"`
    - send-scheduler 请求：`"send.schedule-request"`
  - 返回订阅 ID，可用于后续取消订阅和轮询事件
  - 支持通配符 `"*"` 后缀匹配（如 `"scheduler.control.*"`）
- `eventbus.unsubscribe(subscription_id: string) -> result<_, string>`
  - 取消订阅
- `eventbus.poll-events(subscription_id: string, max_events: u32) -> result<list<event>, string>`
  - 轮询获取订阅的事件（非阻塞，返回已就绪的事件列表）
  - 注意：由于 WIT 不支持真正的 stream，使用轮询模式实现事件订阅
- `eventbus.wait-events(subscription_id: string, max_events: u32, timeout_ms: u32) -> result<list<event>, string>`
  - （async 模式关键接口）允许 scheduler 在事件为空时阻塞等待一段时间
  - `timeout_ms == 0` 等价于 `poll-events`
  - 用途：让 scheduler 的主循环在 idle 时“睡眠”，由事件到达唤醒，从而实现 async/低 CPU
- `eventbus.publish(event: Event) -> result<_, string>`
  - scheduler 向外部发布：
    - 状态变更事件（user/task/action）
    - 拓扑变更结果
    - 统计/告警事件

actions-executor 同样可以通过 eventbus（或通过 scheduler 暴露的简化接口）上报执行结果和观测数据。

### 4. 事件 schema 规范与关键事件类型

为统一后续观测、审计与回放，这里约定所有跨组件事件在语义上遵守统一 schema（具体序列化格式可以是 JSON / WIT record 等）：

- **统一字段（逻辑层面）**：
  - `kind: string`：事件类型标识，例如：
    - `"scheduler.action-result"`
    - `"packet.tx-request"`
    - `"packet.rx"`
    - `"scheduler.state-changed"`
  - `user_id: option<string>`：可选，归属的 user
  - `task_id: option<string>`：可选，归属的 task
  - `action_id: option<string>`：可选，归属的 action
  - `timestamp_ms: u64`：事件产生时间（毫秒）
  - `correlation_id: option<string>`：关联一次调用链路或会话的 ID，便于 end-to-end tracing
  - `payload: string`：业务负载或控制信息，推荐使用 JSON 编码，内部结构由具体事件类型解释

- **关键事件类型一览（示例，不限于此）**：
  - `scheduler.action-result`：
    - 在当前实现中可通过 `eventbus.emit_scheduler_action_result(...)` 辅助函数生成，或由 scheduler 在处理 WIT `ActionOutcome` 后手动构造并 `publish`
    - `kind = "scheduler.action-result"`
    - `payload` 中包含 `status` / `detail` 等（可按需扩展 metrics/exports）
  - `packet.tx-request`：actions-executor 发起一次 UDP 发包意图：
    - `payload` 中至少包含：`sock_id`、`payload`、`user_id?`、`task_id?`、`action_id?`
    - scheduler 解析后调用 host `udp-socket-control` 实际发包，并建立 `sock_id -> 上下文` 映射
  - `packet.rx`：scheduler 从共享内存 RX ring 中解析出一条 UDP 包后生成：
    - `payload` 中至少包含：`sock_id`、`payload_hex` 或 `payload`、`len`
    - 同时在事件顶层字段或 payload 中携带 `user_id?`、`task_id?`、`action_id?`，用于驱动状态机
  - `scheduler.state-changed`：scheduler 自身状态变化（`Idle/Running/Degraded/Error/Completed`）时上报
  - `topology.changed`：workflow / workbook 发生拓扑变更时上报，payload 携带变更 diff
- `scheduler.task.state-changed`：task 状态迁移时上报，payload 携带 from/to、scenario_version 等（便于审计与回放）

> 注：以上事件类型不强制绑定具体字符串前缀，但建议在实现中统一使用 `"scheduler.*"` / `"packet.*"` / `"topology.*"` 命名空间，便于过滤与订阅。

### 4. async 模式的事件闭环（推荐最小闭环）

为了让 scheduler/eventbus 在 WAC + wasmtime 下具备可执行的 async 行为，建议最小闭环如下：

1. scheduler 在 `run()` 启动时订阅（与当前实现对齐）：
  - `scheduler.control.*`（控制）
  - `packet.tx-request`（发包请求）
  - `send.schedule-request`（延迟/定时发送请求）
  - `scheduler.action-result`（action 结果）
  - `packet.rx`（收包事件）
  - `scheduler.timer.*`（定时器）
  - `scheduler.user.*`（用户生命周期）
  - `topology.changed`（拓扑变更）
2. scheduler 主循环：
  - 对控制订阅使用 `wait-events(timeout>0)` 作为**阻塞点**
  - 其他订阅使用 `wait-events(timeout=0)` 做非阻塞 drain
3. host：
  - NIC RX 收包后调用 `packet-ingest.notify-rx(..)` 将共享内存中的数据注入；scheduler 解析后发布 `packet.rx`
  - 对 `packet.tx-request` 事件执行真实发包（或由 scheduler 调用 host 导入直接发包，二选一，但必须产生日志/事件可观测）

这个闭环满足：组件内单线程、host 并行驱动、事件可回放、CPU 不忙等。

---

五、配置模型：workflow / workbook / actions / 负载
-----------------------------------------------

配置文件建议以 **单一 Scenario 配置** 为核心入口，包含：

- `workflow`: 描述任务的执行顺序和依赖关系（状态机拓扑）
- `workbook`: 描述执行所需的资源模型、IP 池等
- `actions`: 描述每种 action 的调用细节
- `load`: 用户上线模型（负载产生和控制）
- `user_resources`: 用户资源绑定模型

### 1. 示例结构（伪 YAML，仅示意字段）

```yaml
version: "v1"
name: "example-scenario"

workbook:
  resources:
    - id: "http-target"
      type: "http-endpoint"
      properties:
        base_url: "http://127.0.0.1:8080"
  ip_pools:
    - id: "pool-1"
      name: "default-pool"
      ranges: ["192.168.0.1-192.168.0.254"]

actions:
  actions:
    - id: "http-get-root"
      call: "GET"
      with:
        url: "{{ resource.http-target.base_url }}/"
        headers:
          User-Agent: "ntx-scheduler"
        # body: ... 可选

workflows:
  nodes:
    - id: "start"
      type: "action"
      action: "http-get-root"
      edges:
        - to: "end"
          trigger:
            condition: "status == 200"
          label: "success"

    - id: "end"
      type: "end"

load:
  ramp_up:
    phases:
      - at_second: 0
        spawn_users: 10
      - at_second: 60
        spawn_users: 100
  user_lifetime:
    mode: "loop"
    iterations: 100
    think_time: "200ms"

user_resources:
  ip_binding:
    enabled: true
    # pool_id 对应 host resources 的 pool 名称（示例：default），用于动态分配 local_ip/local_mac/local_port
    pool_id: "default"
    strategy: "per_user"     # per-user / per-task / shared
    release_on: "user_exit"  # task-end / user-exit
```

#### 1.0.1 Action Node 的 step 配置（推荐形态，对齐当前实现）

除 `action`（单 action）与 `actions[]`（多 action 列表）外，推荐使用 `steps[]` 表达更完整的执行语义：
- step 级 `timeout_ms` / `retry` 覆写（优先于 `actions.actions[*].with.timeout-ms` / `with.retry`）
- step 级失败/超时跳转：`on_failed_step` / `on_timeout_step`

```yaml
workflows:
  nodes:
    - id: "start"
      type: "action"
      steps:
        - action: "a1"
          timeout_ms: 1000
          retry: { max: 2, backoff_ms: 200 }
          on_failed_step: 2
        - action: "a2"
          timeout_ms: 3000
        - action: "a3"
          # timeout_ms/retry 可省略
      edges:
        - to: "end"
          label: "done"
```

说明：
- `steps` 优先级最高；若存在 `steps`，则忽略同节点的 `actions/action`
- `on_failed_step/on_timeout_step` 为 step 索引（从 0 开始），用于在 action-node 内部跳转

### 1.1 UDP Echo 最小场景示例（结合当前实现）

在当前实现下，可以以一个极简的 UDP Echo Client 场景验证事件与状态机链路是否闭环：

```yaml
version: "v1"
name: "udp-echo-minimal"

workbook:
  resources:
    - id: "udp-target"
      type: "udp-endpoint"
      properties:
        peer_ip: "10.0.0.2"
        peer_port: 8080
        # peer_mac 可选：
        # - 如果 host 侧 ARP cache 已有条目，scheduler 会调用 resources.resolve-peer-mac(peer_ip) 自动解析
        # - 如果没有条目，会返回 not-found；此时请显式填写 peer_mac
        peer_mac: "aa:bb:cc:dd:ee:ff"
        # 可选：从哪个资源池分配 local ip/mac/udp-port（默认 "default"）
        pool: "default"

actions:
  actions:
    - id: "udp-send-reply"
      call: "udp.send-reply"
      with:
        payload: "hello-ntx"
        # 可选：超时与重试（毫秒）
        timeout-ms: 3000
        retry:
          max: 0
          backoff_ms: 500

workflows:
  nodes:
    - id: "start"
      type: "action"
      action: "udp-send-reply"
      edges:
        - to: "wait-echo"
          label: "sent"

    - id: "wait-echo"
      type: "wait"
      # 该节点对应一个处于 Waiting 状态的 task，等待对应的 packet.rx
      on:
        event: "packet.rx"
        match:
          action_id: "udp-send-reply"
          # 可选：根据 payload_hex / len 做更精细的过滤
      edges:
        - to: "end"
          label: "echo-ok"

    - id: "end"
      type: "end"

load:
  ramp_up:
    phases:
      - at_second: 0
        spawn_users: 1
  user_lifetime:
    mode: "once"
    # P4：每 user 并发上限（Running task 数）
    max_concurrency: 1

user_resources:
  ip_binding:
    enabled: true
    # host resources 的 pool 名称（一般就是 "default"）
    pool_id: "default"
```

对应执行过程（与前文状态机示例一致）：

1. user 进入 workflow 的 `start` 节点，scheduler 调度 `udp-send-reply`，actions-executor 执行并发布 `packet.tx-request`。
2. scheduler 解析 `packet.tx-request`，调用 host 发包，并在 `sock_ctx` 中记录 `{sock_id, user_id, task_id, action_id}`。
3. host 收到 echo 回复后，通过共享内存 + `packet-ingest.notify-rx` 通知 scheduler，scheduler 解析 RX ring，生成 `packet.rx` 事件并携带 user/task/action 上下文。
4. 状态机在 `wait-echo` 节点上收到与该 task 匹配的 `packet.rx` 事件，将 task 从 `Waiting` 迁移到 `Completed`，并沿 workflow 边进入 `end` 节点，整个场景完成。

### 2. 校验要求（静态）

在 scheduler 加载配置时需要进行：

- actions 引用校验：workflow 中引用的 `action` 必须存在于 `actions` 区域
- 节点引用校验：边 `to` 必须指向存在的节点
- IP 池 / 资源引用校验：user 资源绑定策略引用的 `pool_id`、资源 ID 必须在 workbook 中存在

---

六、scheduler 详细设计
----------------------

### 1. 内部核心模块（逻辑视图，面向万级 user/task）

- **ScenarioManager**
  - 负责加载/解析/校验 scenario 配置（workflow + workbook + actions + load）
- **StateMachineEngine**
  - 根据 workflow 构建静态拓扑（nodes + edges + triggers）
  - 在运行时，维护每个 user 的 task 实例状态
  - 提供：
    - `apply_event(e)`：对状态机应用事件
    - `collect_ready_tasks()`：根据状态/触发条件选出就绪任务  
      （**注意：不能每次遍历全部 task，而是只更新与当前事件相关的少量 task**）
    - `update_topology(change)`：处理拓扑控制事件
- **SchedulerCore**
  - 实现调度主循环逻辑
  - 调用 StateMachineEngine 获取就绪任务，并依据优先级和并发限制进行选取
  - 将任务下发到 actions-executor，并将结果事件回写到状态机
- **LoadController**
  - 根据 `load` 配置，按秒/区间控制 user 的创建和销毁
  - 结合 `user_lifetime` 和 `user_resources`，在合适的时间释放资源
- **ResourceBinder**
  - 使用 `workbook` 中的资源/IP 池信息，为新 user 或新 task 分配资源
  - 提供：`allocate(user_id, task_id)` / `release(user_id, task_id)` 等接口

> 以上所有模块共享同一个单线程事件循环，通过合理的数据结构支撑**万级 user / task** 的高并发管理。

### 2. 调度策略与数据结构（优先级 + 高并发）

- 每个 task 节点可带有：
  - `priority`：用于区分高/中/低优先级任务
  - `weight`：当有大量就绪任务时，用于做加权轮询
- scheduler 维护：
  - **多级就绪队列**（global runnable queues）：
    - 按 `priority` 分桶，例如：高 / 中 / 低
    - 每个桶内部可使用环形队列或加权轮询结构
  - **用户级并发限制**：例如每个 user 同时最多 N 个 Running 任务
  - **组件级并发限制**：针对 actions-executor 实例的最大并发数
  - **快速索引结构**：
    - `user_id -> 用户状态与 task 列表`（如 `IndexMap` / 哈希表）
    - `task_id -> task 实例`（便于根据事件快速定位）
    - （可选）`action_id -> 相关 task 列表`（用于批量处理）

调度逻辑示例：

- 从高优先级到低优先级轮询，就绪队列中弹出任务
- 对每个候选任务检查：
  - user 当前 Running 数量是否超限
  - 所在 actions-executor 实例是否有空闲配额
- 符合条件则下发，否则保留在队列中等待下一轮

> **关键点**：任何一次事件处理和调度循环，都应只访问「与该事件相关的少量 user/task」，而不是线性遍历所有实例，从而保证在万级 user / task 场景下仍然具有良好延迟。

### 3. 事件驱动下的高并发处理流程

在高并发场景下，推荐的事件处理伪代码：

```text
on_event(e):
    // 1. 基于 id 快速定位 user / task
    user = users.get(e.user_id)
    task = tasks.get(e.task_id)

    // 2. 在本地更新与该 event 相关的少量状态
    apply_event_to_state_machine(user, task, e)

    // 3. 若某些 task 切换为 Ready → 放入对应 priority 的 runnable 队列
    if task.state == Ready:
        runnable_queues[task.priority].push(task.id)

    // 4. 在当前调度配额范围内，从 runnable_queues 中取若干 task 派发执行
    while has_dispatch_capacity():
        t = pop_next_runnable_task()
        if t is None:
            break
        dispatch_to_actions_executor(t)
```

### 4. 计时器与批量事件（Think-time / 超时 / 上下线）

为支撑成千上万 user 的 think-time、超时控制与上线模型，TimerManager 建议：

- 采用 **分层时间轮 / 最小堆** 维护未来一段时间内的 timer：
  - 存储 `deadline -> [timer_id...]` 映射
  - 到期时批量生成 `TimerEvent` 注入事件队列
- 用户上线模型：
  - LoadController 不直接一次性创建大量 user，而是按照 `ramp_up` 的配置，在每个 tick 内将「要创建的 user 数」拆成若干小批次，逐步注入事件队列：

```text
on_tick(now):
    // 计算本 tick 需要新增的 user 数量
    n = calc_users_to_spawn(now)
    for i in 0..n:
        publish_event(UserStartRequested { ... })
```

这样可以保证：

- scheduler 每次处理的事件规模可控
- 用户创建/销毁分散到时间轴上，避免瞬间打满单线程循环

---

七、actions-executor 详细设计
-----------------------------

### 1. 角色与边界

- **不负责调度，仅负责执行**：
  - 不关心 workflow 拓扑，也不关心 user 生命周期
  - 只接收「要执行的 action（或 task 内 action 集合）」和必要上下文（资源绑定、变量等）
- **组件内部抽象**：
  - **不直接与 host 交互**：actions-executor 不能直接导入或调用 host 提供的接口（如 `ntx:host/udp-socket-control`、`ntx:host/resources`）
  - **通过 eventbus 和 scheduler 间接与 host 通信**：
  - 需要发包时，通过 `eventbus.publish()` 发布 `packet.tx-request` 事件，由 scheduler 处理并调用 host 的 `ntx:host/udp-socket-control`（如 `tx()`）
    - 需要资源时，通过 scheduler 在 workflow 初始化/节点推进阶段完成资源绑定，并通过 `ActionContext` / `vars` 等上下文传入 action（executor 内不直接申请资源）
  - 收包通知通过 eventbus 的 `packet.rx` 事件接收，由 scheduler 在 host 通过 `packet-ingest.notify-rx(..)` 注入刺激后解析共享内存并发布
  - **对 scheduler 提供统一的执行结果模型 `ActionOutcome`**：
    - `status`：Success / Failed / Timeout / Skipped …
    - `detail`：可读字符串（用于日志）
    - （可选）metric 字段：如 `latency_ms`、`bytes_sent/received` 等
  - **不暴露底层 socket / HTTP 实现细节给 scheduler**：scheduler 只关心 action 的执行结果，不关心具体的网络协议实现

### 1.1 强约束：非阻塞 / 等待 / 超时 / 重试 的推荐范式（必须遵守）

> 本系统在 wasm32-wasip2 下以**单线程事件驱动**为第一原则；因此“等待”不应发生在 actions-executor 内部。

#### 核心原则

- **executor 不做自旋等待**：禁止在 `execute-action()` 内部通过 loop/poll 的方式等待 `packet.rx` / 外部回调 / 定时器等（包括忙等、重复 poll-events、sleep 等）。
- **等待由 scheduler 的状态机节点推进**：
  - action 负责“发起副作用”（发布 `packet.tx-request`、`send.schedule-request` 等）并**立即返回**
  - workflow 中使用 `wait` 节点（或等价机制）消费 `packet.rx` 等事件，推动 task 从 `Waiting -> Completed/Failed`
- **超时由 scheduler 的 timer event 驱动**：超时/重试/think-time 统一由 scheduler 的 TimerManager 生成 `timer-fired`（或等价事件）注入事件队列，由状态机决定迁移与重试，而不是 executor 内部计时与循环。

#### 推荐流程（以 UDP Echo client 为例）

```text
execute-action(udp.send)  -> publish(packet.tx-request) -> return Success
                          scheduler 处理 tx 并建立 sock_ctx
host 收到包 -> notify-rx -> scheduler 解析 ring -> publish(packet.rx)
workflow wait 节点消费 packet.rx -> 状态机迁移 Waiting -> Completed
若超时：TimerFired -> 状态机迁移 Waiting -> Failed/Ready(重试)
```

#### 允许与不允许（实践口径）

- **允许**
  - executor：构造事件/委托请求（`packet.tx-request` / `send.schedule-request`）并返回 `ActionOutcome`
  - scheduler：订阅/轮询 eventbus、解析 RX ring、投递 `packet.rx`、发出 timer event、驱动状态机
- **不允许**
  - executor：在一次 `execute-action` 中“等到某个事件发生”为止（即便有 deadline），这会导致单线程被占用并放大尾延迟

> 现状提示：如果仓库里存在 `udp.send-recv` 这类在 executor 内部订阅并轮询 `packet.rx` 的实现，它应视为**过渡方案**，优先按上述范式迁移到“wait 节点 + timer event”模型。

### 2. 建议的接口能力（WIT 抽象）

仅列出功能，不限制具体签名：

- `init_component() -> Result<()>`
  - 组件生命周期初始化，准备内部状态（如有）
- `execute_action(action_def) -> Result<ActionOutcome>`
  - `action_def`：来自 `actions` 配置的定义（method、url 模板、headers、body 模板等），已由 scheduler 进行模板展开
  - 执行过程中如需发包，通过 `eventbus.publish(packet.tx-request)` 委托给 scheduler
  - **不在 executor 内等待收包**：等待/超时/重试由 scheduler 的状态机（`wait` 节点 + timer event）推进；executor 仅负责发起副作用并返回
  - 返回执行结果 `ActionOutcome`
- `release_component() -> Result<()>`
  - 在场景结束或不再需要时释放资源

对于需要并行执行 action 的情况，可以：

- 在 host 层创建多个 actions-executor wasm 实例，由 scheduler 选择具体实例
- 或在 actions-executor 内部采用「一次调用执行多个动作」的批量接口（仍然遵循单线程事件驱动）

---

八、WIT 接口设计（针对 UDP Echo 场景）
------------------------------------

### 1. scheduler → actions-executor 接口

**接口位置**：`component/wit/actions-executor/world.wit`

```wit
package ntx:scenario-actions-executor@0.1.0;

// 说明：当前实现已引入 core-types 模块来承载跨组件共享 types，
// 并废弃 send-scheduler，统一通过 event-bus 发布请求/结果事件来闭环。
use ntx:scenario-types/types@0.1.0 as t;
use ntx:scenario-eventbus/event-bus@0.1.0;

/// actions-executor：只负责执行 action，不负责调度。
interface action-component {
    use t.{action-def, action-outcome, action-context};

    init-component: func() -> result<_, string>;
    
    /// 执行一个 action
    /// - action: 来自配置的 action 定义，包含 call 类型和参数（已由 scheduler 进行模板展开）
    /// - ctx: 运行时上下文（user/task/action 关联信息、资源绑定、变量等）
    /// - 返回: action 执行结果（成功/失败状态 + 详情）
    /// - 注意：执行过程中如需发包，应通过 eventbus 发布 packet.tx-request 事件，由 scheduler 处理
    execute-action: func(action: action-def, ctx: option<action-context>) -> result<action-outcome, string>;
    
    release-component: func() -> result<_, string>;
}

world action-executor-component {
    import event-bus;
    export action-component;
}
```

### 2. scheduler → host 的 UDP Socket 接口（actions-executor 间接使用）

**接口位置**：`component/wit/host/world.wit`（由 scheduler 导入；该文件内包含 `udp-socket-control` 与 `resources` 两个 interface）

**重要说明**：
- 这些接口由 **scheduler 直接导入**，用于与 host 通信
- **actions-executor 不能直接导入或调用这些接口**
- actions-executor 如需发包，应通过 `eventbus.publish(packet.tx-request)` 事件委托给 scheduler
- scheduler 接收到 `packet.tx-request` 事件后，调用这些接口完成实际的网络操作

**核心接口说明**：

```wit
package ntx:host;

interface udp-socket-control {
    use types.{ipv4-addr, mac-addr};
    type resource-id = string;
    type sock-id = u64;

    /// UDP socket 句柄
    record udp-socket {
        owner: resource-id,  // 资源所有者 ID（通常为 user_id）
        sock: sock-id,        // socket 内部 ID
    }

    /// UDP socket 绑定配置（一次性配置）
    record udp-bind {
        local-ipv4: ipv4-addr,      // 本地 IPv4（从资源池分配）
        local-mac: mac-addr,         // 本地 MAC
        local-udp-port: u16,         // 本地 UDP 端口（从资源池分配）
        peer-ipv4: ipv4-addr,         // 对端 IPv4
        peer-port: u16,               // 对端 UDP 端口
        peer-mac: mac-addr,           // 对端 MAC
        ttl: option<u8>,              // 可选 TTL
    }

    /// 帧句柄（指向 host 管理的共享内存中的帧数据）
    record frame-handle {
        region: u32,
        offset: u32,
        len: u32,
    }

    /// 创建 UDP socket
    create: func(name: string) -> result<udp-socket, socket-error>;
    
    /// 绑定 UDP socket（配置本地/对端地址）
    bind: func(sock: sock-id, b: udp-bind) -> result<_, socket-error>;
    
    /// 构建 UDP 回复帧并写入共享内存
    /// - payload: UDP payload 数据
    /// - 返回: 指向共享内存中帧数据的句柄
    build-reply: func(sock: sock-id, payload: list<u8>) -> result<frame-handle, socket-error>;
    
    /// 发送帧（通过 frame-handle）
    /// - 返回: 实际发送的字节数
    tx: func(frame: frame-handle) -> result<u32, socket-error>;
}
```

**资源管理接口**（同样位于 `component/wit/host/world.wit`，由 scheduler 导入）：

```wit
package ntx:host;

interface resources {
    use types.{ipv4-addr, mac-addr};
    type resource-id = string;
    type owner-id = resource-id;

    /// 创建 socket owner（通常对应一个 user）
    create-socket-owner: func(name: string) -> result<owner-id, resource-error>;
    
    /// 从资源池分配 UDP “身份”：local IPv4 + MAC + UDP port（供 scheduler 绑定 socket）
    record udp-identity {
        local-ipv4: ipv4-addr,
        local-mac: mac-addr,
        local-udp-port: u16,
    }
    acquire-udp-identity: func(pool: string, owner: owner-id) -> result<udp-identity, resource-error>;
    
    /// 解析资源 ID 对应的 UDP 端口
    resolve-udp-port: func(rid: resource-id) -> result<u16, resource-error>;
}
```

### 3. UDP Echo Client/Server 的 Action 定义

**在配置文件中，UDP echo 的 action 定义示例**：

```yaml
actions:
  actions:
    # UDP Echo Client Action
    - id: "udp-echo-client"
      call: "udp.send-recv"  # action 类型标识
      with:
        # 目标服务器地址（从 resource 或变量中获取）
        peer-ip: "{{ resource.target-server.ip }}"
        peer-port: 8080
        # 本地绑定（从 user 资源池获取）
        local-ip: "{{ user.resources.ip }}"
        local-port: "{{ user.resources.udp-port }}"
        # payload 内容（可以是模板）
        payload: "{{ action.payload }}"
        # 超时配置
        timeout-ms: 5000
        # 期望的 echo 回复（用于验证）
        expect-echo: true

    # UDP Echo Server Action
    - id: "udp-echo-server"
      call: "udp.receive-echo"  # action 类型标识
      with:
        # 监听地址
        bind-ip: "0.0.0.0"  # 或从 resource 获取
        bind-port: 8080
        # 处理模式
        mode: "echo"  # echo: 原样返回, transform: 可自定义处理
        # 最大接收大小
        max-payload-size: 65507
```

### 4. actions-executor 内部实现流程（UDP Echo Client）

**伪代码流程**：

```text
execute_action(action: ActionDef, ctx: Option<ActionContext>) -> ActionOutcome:
    // 1. 从 ctx 获取上下文信息
    user_id = ctx.user_id
    task_id = ctx.task_id
    action_id = ctx.action_id
    
    // 2. 解析 action 参数（params 是 JSON 字符串）
    params = parse_json(action.params)
    peer_ip = params["peer-ip"]
    peer_port = params["peer-port"]
    local_ip = params["local-ip"]  // 从 user 资源获取
    local_port = params["local-port"]  // 从 user 资源获取
    payload = params["payload"]
    
    // 3. 通过 scheduler/host 已建立的资源绑定拿到 socket 上下文（此处简化为已有 sock_id）
    sock_id = resolve_user_socket(user_id, peer_ip, peer_port)
    
    // 4. 构造发包意图（不直接调用 host）
    tx_req = {
        sock_id: sock_id,
        payload: payload,
        user_id: user_id,
        task_id: task_id,
        action_id: action_id,
    }
    
    // 5. 通过 eventbus / scheduler WIT 将发包请求交给 scheduler/host 执行
    publish_event(PacketTxRequestEvent {
        kind: "packet.tx-request",
        payload: json_encode(tx_req),
    })
    
    // 6. 等待接收 echo 回复（通过 eventbus 或轮询机制）
    // 注意：在 wasm-wasip2 单线程下，需要通过事件驱动接收
    // 这里简化描述，实际需要通过 host 的事件机制接收
    
    // 7. 验证回复（如果是 echo client）
    if expect_echo:
        if received_payload == payload:
            return ActionOutcome { 
                status: Success, 
                detail: Some("echo matched"),
                metrics: None,
                exports: None,
            }
        else:
            return ActionOutcome { 
                status: Failed, 
                detail: Some("echo mismatch"),
                metrics: None,
                exports: None,
            }
    
    return ActionOutcome { 
        status: Success,
        detail: None,
        metrics: None,
        exports: None,
    }
```

### 5. actions-executor 内部实现流程（UDP Echo Server）

**伪代码流程**：

```text
execute_action(action: ActionDef, ctx: Option<ActionContext>) -> ActionOutcome:
    // 1. 解析 action 参数（params 是 JSON 字符串）
    params = parse_json(action.params)
    bind_ip = params["bind-ip"]
    bind_port = params["bind-port"]
    mode = params["mode"]  // "echo" 或其他
    
    // 2. 创建 socket owner 和分配端口
    owner_id = resources.create-socket-owner("udp-echo-server")
    resources.acquire-udp-port("server-pool", owner_id)
    
    // 3. 创建并绑定 UDP socket（server 模式，peer 地址为 0.0.0.0:0）
    socket = udp-socket-control.create("udp-echo-server")
    bind_config = udp-bind {
        local-ipv4: bind_ip,
        local-udp-port: bind_port,
        peer-ipv4: 0.0.0.0,  // server 模式，不指定 peer
        peer-port: 0,
        ...
    }
    udp-socket-control.bind(socket.sock, bind_config)
    
    // 4. 进入接收循环（通过事件驱动）
    // 注意：在 wasm-wasip2 下，需要通过 host 的事件机制接收数据包
    // 当收到数据包时：
    //   - 解析 payload
    //   - 根据 mode 处理（echo 模式：原样返回）
    //   - 构建回复帧并发送
    
    // 5. 返回执行结果
    return ActionOutcome { 
        status: Success, 
        detail: Some("server started"),
        metrics: None,
        exports: None,
    }
```

### 6. 关键设计要点

1. **L4 及以下封装在 host**：
   - actions-executor 不直接操作 L2/L3 层（MAC、IP 头构造）
   - 通过 `udp-socket-control.build-reply` 和 `tx`，host 负责封装完整的 UDP/IP/Ethernet 帧
   - actions-executor 只需提供 **UDP payload** 数据

2. **资源管理与 host 能力隔离**：
   - IP 地址、MAC 地址、UDP 端口等资源由 host 的 `resources` 接口管理
   - scheduler 在创建 user 时，通过 resource-manager / host WIT 分配资源，并维护与 user/task 的绑定关系
   - **actions-executor 不直接导入 host 接口**（如 `udp-socket-control`、`resources`），只通过：
     - scheduler 暴露的 WIT 接口（如发包调度、packet-tx），以及
     - eventbus 上的业务/控制事件
     
     来间接驱动 host 行为；所有对 host 的副作用都需经过 scheduler 的编排与审计
   - 对于 UDP socket，scheduler 维护一张运行时映射表：`sock_id -> { user_id, task_id, action_id, last_seen_ms }`：
     - 在处理发包请求（如 `packet.tx-request`）时写入/刷新此映射
     - 在收包时根据 `sock_id` 查表，将网络事件映射回具体 user/task
     - 在 **socket 关闭** 或 **user 生命周期结束** 时，从映射表中清理对应条目，避免内存泄漏和上下文“脏挂载”

3. **UDP 收包通知机制（统一经由 scheduler）**：

   - host 在收到 UDP 包后，将数据包放入共享内存，并通过 **事件/回调机制** 唤醒 scheduler 组件
   - scheduler 导出 `packet-ingest.notify-rx(desc-mem, payload-mem)` 接口，host 调用该接口：
     - scheduler 内部参考 packet-engine 的 `drain_rx_ring` 逻辑解析 RX ring：
       - 校验 `MAGIC` / `VERSION` / `head` / `tail` / `desc_capacity`
       - 批量读取若干描述符（每轮最多 N 条），解析出 `(sock_id, payload_off, payload_len)`
       - 从 payload buffer 切片出 UDP payload
     - 依据 `sock_id` 在运行时映射表中查找到对应的 `{user_id, task_id, action_id}` 上下文，并刷新该 sock 的 `last_seen_ms`（便于后续资源清理与观测）
     - 为每个数据包构造 `packet.rx` 事件并通过 eventbus 发布，用于驱动状态机中对应 task 的状态转移（例如 `Waiting → Ready`）
       - `packet.rx` 的 payload 中包含：
         - `sock_id`: 触发该事件的 socket
         - `seq`: 单调递增的包序号（per-process），便于调试排序
         - `len`: UDP payload 长度
         - `payload_hex`: UDP payload 的十六进制编码（避免二进制污染日志）
         - `ts_ms`: scheduler 解析该包时的时间戳
   - actions-executor 不直接参与收包轮询，只消费由 scheduler 转译后的事件或由 scheduler 再次调度的 action。

4. **action 参数模板化**：
   - action 定义中的 `with` 字段支持变量模板（如 `{{ user.resources.ip }}`）
   - scheduler 在调用 `execute-action` 前，先进行模板展开
   - actions-executor 接收到的 `action-def` 已经是展开后的具体值

5. **发包机制：统一通过 eventbus 委托 scheduler 发送**：

   - **actions-executor 不直接调用 host 接口**，所有发包请求都通过 `eventbus.publish(packet.tx-request)` 事件委托给 scheduler
   - `packet.tx-request` 事件包含：
     - `sock_id`：目标 socket ID（由 scheduler 在创建 socket 时分配）
     - `payload`：UDP payload 数据（JSON 编码）
     - `user_id`、`task_id`、`action_id`：上下文信息（用于 scheduler 更新 `SockCtx` 映射）
   - scheduler 接收到 `packet.tx-request` 事件后：
     - 调用 host 的 `udp-socket-control.build-reply()` 构建回复帧
     - 调用 host 的 `udp-socket-control.tx()` 发送数据包
     - 更新 `SockCtx` 映射表（`sock_id -> {user_id, task_id, action_id, last_seen_ms}`）
   - 对于周期性发包、速率控制（PPS）、批量发送等场景，scheduler 可以维护「发包调度队列」，并以 event-bus 事件形式接收/输出：
     - actions-executor 发布 `send.schedule-request`（或等价事件）提交调度请求
     - scheduler 发布 `send.scheduled` / `send.tick` / `send.completed`（或等价事件）用于观测与回放
   
   **发包委托数据结构**：
   ```wit
   record send-request {
       // 标识信息
       request-id: string,           // 唯一标识此发包请求
       task-id: string,              // 关联的 task（用于状态机更新）
       user-id: string,               // 关联的 user
       
       // Socket 信息
       socket-id: u64,               // UDP socket ID
       
       // 发送策略
       schedule: send-schedule,      // 发送时间表
       
       // Payload 配置（payload 和 payload-generator 二选一，两者同时存在时 host/scheduler 自行裁决）
       payload: option<list<u8>>,     // 固定 payload（可选）
       payload-generator: option<payload-generator>,  // 动态生成 payload（可选）
       
       // 生命周期控制
       max-count: option<u32>,        // 最大发送次数（None = 无限）
       timeout-ms: option<u64>,      // 超时时间（None = 不超时）
   }
   
   variant send-schedule {
       // 立即发送一次
       once,
       
       // 固定间隔周期性发送
       periodic(periodic-schedule),
       
       // 按时间表发送（支持复杂调度）
       timetable(timetable-schedule),
       
       // 速率控制发送（PPS：packets per second）
       rate-limited(rate-limited-schedule),
   }
   
   record periodic-schedule {
       interval-ms: u64,         // 发送间隔（毫秒）
       start-delay-ms: option<u64>,  // 首次发送延迟（可选）
   }
   
   record timetable-schedule {
       timestamps-ms: list<u64>,     // 发送时间戳列表（毫秒，相对于请求创建时间）
   }
   
   record rate-limited-schedule {
       pps: u32,                  // 每秒包数
       burst-size: option<u32>,   // 突发大小（可选，用于令牌桶）
   }
   
   // Payload 生成器（支持动态 payload）
   variant payload-generator {
       // 固定 payload
       fixed(fixed-payload),
       
       // 序列号 payload（每次发送递增）
       sequence(sequence-payload),
       
       // 时间戳 payload
       timestamp(timestamp-payload),
   }
   
   record fixed-payload {
       payload: list<u8>,
   }
   
   record sequence-payload {
       template: list<u8>,        // payload 模板（可包含 {{seq}} 占位符）
       start-seq: u32,            // 起始序列号
   }
   
   record timestamp-payload {
       template: list<u8>,         // payload 模板（可包含 {{timestamp}} 占位符）
   }
   ```
   
   **scheduler 发包调度器设计**：
   
   ```text
   // Scheduler 内部维护发包调度队列
   struct SendScheduler {
       // 按时间排序的发送队列（最小堆）
       send_queue: BinaryHeap<SendRequest>,
       
       // 按 request-id 索引的活跃请求
       active_requests: HashMap<String, SendRequest>,
       
       // 速率限制器（按 socket-id 分组）
       rate_limiters: HashMap<u64, TokenBucket>,
   }
   
   // 在主循环中处理发包
   on_tick(now):
       // 1. 检查发送队列，取出到期的请求
       while let Some(req) = send_queue.peek() {
           if req.next_send_time <= now {
               // 2. 生成 payload（如果使用 generator）
               payload = generate_payload(req)
               
               // 3. 构建 UDP 帧
               frame_handle = udp-socket-control.build-reply(req.socket-id, payload)
               
               // 4. 检查速率限制
               if rate_limiter.check(req.socket-id):
                   // 5. 发送
                   udp-socket-control.tx(frame_handle)
                   
                   // 6. 更新请求状态
                   update_send_request(req)
                   
                   // 7. 如果还有剩余次数，重新入队
                   if req.remaining_count > 0:
                       req.next_send_time = calculate_next_send_time(req)
                       send_queue.push(req)
                   else:
                       // 完成，发送完成事件
                       eventbus.publish(SendRequestCompleted {
                           request-id: req.request-id,
                           task-id: req.task-id,
                           total-sent: req.total-sent,
                       })
               else:
                   // 速率限制，延迟发送
                   req.next_send_time = now + calculate_backoff(req)
                   send_queue.push(req)
           else:
               break  // 队列已按时间排序，后续请求未到期
       }
   ```
   
   **actions-executor 委托发包（当前方案：event-bus 事件）**：

   - executor 通过 `eventbus.publish()` 提交调度请求
   - scheduler 在内部维护 send-queue，并通过事件发布状态变化（可观测 + 可回放）

   建议事件（示意，字段可按实现调整）：

   - `send.schedule-request`：提交调度请求（相当于旧的 `schedule-send`）
   - `send.cancel-request`：取消调度请求（相当于旧的 `cancel-send`）
   - `send.status-changed`：调度状态变化（pending/active/paused/completed/failed）
   - `send.completed`：调度完成事件（可携带 total-sent、last-error 等）

   说明：事件 payload 推荐使用 JSON，至少包含 `request_id`，以及用于追踪/归因的 `user_id` / `task_id` / `correlation_id`。

   **（历史参考，已废弃）send-scheduler WIT 接口**：
   
   ```wit
   // 旧方案：在 scheduler 的 WIT 接口中新增，供 executor 直接调用。
   // 现已废弃：统一走 event-bus，避免额外接口面并保持事件可回放。
   interface send-scheduler {
     schedule-send: func(request: send-request) -> result<string, string>;
     cancel-send: func(request-id: string) -> result<_, string>;
     query-send-status: func(request-id: string) -> result<send-status, string>;
   }
   
   record send-status {
       request-id: string,
       state: send-request-state,
       total-sent: u32,
       last-sent-time-ms: option<u64>,    // 上次发送时间（毫秒）
       next-send-time-ms: option<u64>,    // 下次发送时间（毫秒）
       last-error: option<string>,         // 最后一次错误信息（如果有）
   }
   
   enum send-request-state {
       pending,      // 等待发送
       active,       // 正在发送
       paused,       // 已暂停
       completed,    // 已完成
       cancelled,    // 已取消
       error,        // 错误
   }
   ```
   
   **actions-executor 使用示例**：
   
   ```text
   // 在 execute_action 中委托周期性发包
   execute_action(action: ActionDef, ctx: Option<ActionContext>) -> ActionOutcome:
       // 1. 从 ctx 获取上下文信息
       user_id = ctx.user_id
       task_id = ctx.task_id
       
       // 2. 构造发包委托（支持固定 payload 或动态生成）
       send_req = send-request {
           request-id: generate_id(),
           task-id: task_id,
           user-id: user_id,
           socket-id: socket.sock,
           schedule: send-schedule.rate-limited(rate-limited-schedule {
               pps: 100,  // 每秒 100 包
               burst-size: Some(10),
           }),
           // 方式1：使用固定 payload
           payload: Some(b"hello-ntx"),
           payload-generator: None,
           // 或方式2：使用动态生成 payload
           // payload: None,
           // payload-generator: Some(payload-generator.sequence(sequence-payload {
           //     template: b"echo-seq-{{seq}}",
           //     start-seq: 0,
           // })),
           max-count: Some(1000),  // 发送 1000 次
           timeout-ms: Some(30000),  // 30 秒超时
       }
       
       // 3. 委托 scheduler 发送
  // 旧：request_id = scheduler.schedule-send(send_req)
  // 新：publish(send.schedule-request{...}) 并以 request_id/correlation_id 关联后续事件
       
       // 4. 返回（action 执行完成，实际发包由 scheduler 异步进行）
       return ActionOutcome {
           status: Success,
           detail: format!("scheduled send request: {}", request_id),
           metrics: None,
           exports: None,
       }
   ```
   
   **关键设计要点**：
   
   - **解耦发送逻辑**：actions-executor 只负责构造发包委托，scheduler 负责实际调度和发送
   - **支持复杂调度**：支持立即发送、周期性发送、速率控制、时间表等多种模式
   - **动态 payload**：支持序列号、时间戳等动态生成 payload
   - **生命周期管理**：支持取消、暂停、查询状态等操作
   - **事件通知**：发包完成/失败时，通过 eventbus 通知 actions-executor 或更新状态机

### 7. UDP 收包通知的完整流程

#### 收包统一转译模型（Client/Server 通用）

```
┌─────────────────────────────────────────────────────────────┐
│ Host 侧                                                      │
├─────────────────────────────────────────────────────────────┤
│ 1. NIC 收到 UDP 包                                          │
│ 2. 解析 UDP/IP/Ethernet 头                                  │
│ 3. 将 payload 写入共享内存                                  │
│ 4. 将 desc ring / payload 区域编码为两段内存（desc_mem/payload_mem）│
│ 5. 通过组件导出接口调用 guest：`packet-ingest.notify-rx(desc_mem, payload_mem)`│
└─────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────┐
│ Scheduler 主循环（Guest 侧）                                │
├─────────────────────────────────────────────────────────────┤
│ loop {                                                       │
│     // 1. host 的 notify-rx 会触发 scheduler 解析 RX ring，   │
│     //    并由 scheduler 主动 publish `packet.rx` 事件到 eventbus │
│     // 2. scheduler 在主循环中从 eventbus drain `packet.rx`： │
│     //    - 根据 sock_id / sock_ctx 找到对应 Waiting task      │
│     //    - 将收包统一转译为显式事件并驱动状态机推进           │
│     //    - 若推进到 Ready，则进入 runnable queue              │
│                                                              │
│     // 7. 调度就绪的 task                                    │
│     dispatch_ready_tasks()                                  │
│ }                                                           │
└─────────────────────────────────────────────────────────────┘
```

> 说明：本系统的收包链路遵循统一原则：**收包永远先进入 scheduler**，
> 由 scheduler 统一转译为 `packet.rx` 事件，再驱动状态机与后续 action 执行。

##### Server 场景同样适用（推荐）

“Server 场景”和“Client 场景”本质都是**处理收包**，差别在于是否需要**立即生成响应 payload 并发送**。推荐做法是：

1. scheduler 收包 → 生成 `packet.rx`（携带 `sock_id/meta/payload`）事件 → 状态机把对应 task 从 `Waiting` 推进到可执行状态；
2. scheduler 调度一个 server handler action（例如 `udp.echo` / `udp.server.handle`）到 actions-executor 执行；
3. actions-executor 根据收到的 payload 生成响应（echo/transform/业务逻辑），并通过 `eventbus.publish(packet.tx-request)` 委托 scheduler/host 发送（或在 `ActionOutcome.exports` 中返回响应并由 scheduler 转译为 tx-request）；
4. scheduler 统一执行 `udp-socket-control.build-reply + tx` 完成回包，并更新 SockCtx/观测事件。

### 8. 接口依赖关系图

```
scheduler (component)
    │
    ├─→ import action-component (from actions-executor)
    │       └─→ execute-action(action-def, ctx: option<action-context>) → action-outcome
    │
    ├─→ (send scheduling via event-bus)
    │       ├─→ consume send.schedule-request / send.cancel-request
    │       └─→ publish send.status-changed / send.completed
    │
    ├─→ import event-bus
    │       ├─→ subscribe(topic-filter) → subscription-id
  │       ├─→ poll-events(subscription-id, max-events) → list<event>
  │       ├─→ wait-events(subscription-id, max-events, timeout-ms) → list<event>
    │       └─→ publish(event)  // 订阅/发布事件（接收 action-result 事件）
    │
  └─→ import ntx:host/udp-socket-control (用于实际发送)
            ├─→ build-reply(socket-id, payload) → frame-handle
            └─→ tx(frame-handle) → bytes-sent

actions-executor (component)
    │
    ├─→ export action-component
    │       └─→ execute-action(action-def, ctx: option<action-context>) → action-outcome
    │
    └─→ import ntx:scenario-eventbus/event-bus@0.1.0
            ├─→ publish(packet.tx-request)  // 委托发包给 scheduler
            ├─→ subscribe(packet.rx)        // 接收收包事件（可选）
            └─→ poll-events(subscription-id, max-events)  // 轮询订阅的事件
    
  // 注意：actions-executor 不直接导入 host 接口（如 ntx:host/udp-socket-control、ntx:host/resources）
    // 所有与 host 的交互都通过 eventbus 事件和 scheduler 的 WIT 接口间接完成
```

**发包流程（统一通过 eventbus 委托 scheduler）**：

```
┌─────────────────────────────────────────────────────────────┐
│ actions-executor 发包流程（通过 eventbus）                    │
├─────────────────────────────────────────────────────────────┤
│ actions-executor.execute_action()                           │
│   ├─→ 构造 packet.tx-request 事件 {                        │
│   │      sock_id: <socket-id>,                              │
│   │      payload: <udp-payload>,                            │
│   │      user_id: <user-id>,                               │
│   │      task_id: <task-id>,                                │
│   │      action_id: <action-id>,                            │
│   │   }                                                      │
│   └─→ eventbus.publish(packet.tx-request)  ← 委托给 scheduler│
└─────────────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────────┐
│ scheduler 处理 packet.tx-request 事件                       │
├─────────────────────────────────────────────────────────────┤
│ scheduler 主循环（从 eventbus 接收事件）                     │
│   ├─→ 接收 packet.tx-request 事件                          │
│   ├─→ 更新 SockCtx 映射表 (sock_id -> {user_id, task_id, ...})│
│   ├─→ udp-socket-control.build-reply(sock_id, payload)   │
│   └─→ udp-socket-control.tx(frame_handle)  ← 实际发送       │
└─────────────────────────────────────────────────────────────┘

对于周期性发包、速率控制（PPS）、批量发送等场景：
┌─────────────────────────────────────────────────────────────┐
│ actions-executor.execute_action()                           │
│   └─→ 构造 send.schedule-request 事件 payload {             │
│          request_id: "...",                                │
│          socket_id: <socket-id>,                             │
│          schedule: rate-limited { pps: 100 },               │
│          payload_generator: sequence { ... },               │
│          max_count: 1000,                                   │
│          user_id/task_id/correlation_id: "..."             │
│       }                                                      │
│   └─→ event-bus.publish(send.schedule-request)               │
│                                                              │
│ scheduler 主循环                                             │
│   ├─→ 检查发送队列（按时间排序）                            │
│   ├─→ 生成 payload（使用 generator）                        │
│   ├─→ udp-socket-control.build-reply(socket, payload)     │
│   └─→ udp-socket-control.tx(frame_handle)  ← 实际发送       │
└─────────────────────────────────────────────────────────────┘
```

---

九、与外部组件的集成（WIT 维度）
-------------------------------

调度器需要通过 WIT 接口对接的外部能力包括：

- **wasmtime 运行时**
  - 负责加载和运行 scheduler / actions-executor wasm-wasip2 组件
- **eventbus**
  - 事件订阅 / 发布
- **actions-executor**
  - 通过 WIT 接口调用 actions-executor 组件（详见第八节）
- **resource-manager**
  - 资源分配与回收，如 IP、port、用户标签及其他业务资源
- **workflow-manager / task-manager / action-manager / event-manager**
  - 此处可以抽象为「配置/元数据管理服务」
  - scheduler 可从其中拉取最新的 workflow / workbook / actions 定义，或接收拓扑更新事件

上述管理服务也可以在初期收敛为：由 host 直接提供配置文件，再通过 eventbus 发送「变更事件」，不必一开始就拆分成多个独立模块。

---

九、非功能需求与后续演进点
--------------------------

1. **可观测性**
   - 统一 event schema，方便日志聚合与指标统计
   - 对关键错误（资源不足、配置不合法、action 失败率过高等）提供告警事件
2. **可回放**
   - 在设计事件模型时，要考虑后续做「事件重放」以便调试
   - 例如：按照时间顺序重放 action-result 事件，重建状态机演化过程
3. **热更新**
   - 后续可以考虑允许在不中断 scheduler 的情况下：
     - 热更新 workflow 配置
     - 热滚动 actions-executor 实现（新旧版本并存，按 user 或 tenant 进行灰度）

---

十、小结
--------

- scheduler：**事件驱动 + 状态机 + 负载控制 + 资源绑定** 的中枢，运行在 wasm-wasip2 组件中，单线程，通过 eventbus 与外界交互，并作为 host 能力的统一入口（包括 UDP 收包的 ring 解析、`packet.rx` 事件生成，以及对 `packet.tx-request` 的实际发包执行）。
- actions-executor：**专注于执行 action 的 wasm-wasip2 组件**，不能直接与 host 通信，只能通过 scheduler 暴露的 WIT 接口与 eventbus 事件间接驱动 host，对 scheduler 提供统一的执行接口与结果模型。
- 配置层：通过 **workflow + workbook + actions + load + user_resources**，在不修改二进制的前提下描述复杂场景。
- 以上所有数据与行为，均满足「事件是唯一合法动态入口」这一约束，后续实现时可以以本设计为约束，针对 WIT 接口与内部数据结构做进一步细化与代码化。

---

十一、现状完成度 & 里程碑（与实现对齐）
--------------------------------------

> 目的：避免“设计看起来都支持，但实现还没做”的落差。本节以仓库当前代码为准，标注已实现/部分实现/未实现，并给出下一步里程碑。

### 1) actions-executor（`component/actions-executor`）

- **已实现**
  - `init-component / execute-action / release-component` 基本骨架已完成（WIT 对齐 `component/wit/actions-executor/world.wit`）
  - `udp.send` / `udp.send-reply`：通过 `eventbus.publish(kind="packet.tx-request")` 委托 scheduler/host 侧实际发包
  - `udp.send-recv`：**已按事件驱动口径完成**——仅委托发包（发布 `packet.tx-request`）并立即返回（no-wait）；收包等待/超时/重试由 scheduler 的状态机（`wait` 节点 + timer event）推进
  - `udp.schedule-send`：通过 event-bus 发布 `send.schedule-request` 提交发送调度（**固定 payload**）；具体发包由 scheduler 统一处理
- **部分实现（存在待优化点）**
  - （预留）后续可补齐更丰富的 metrics/correlation 透传、以及更多 action 类型的统一错误/重试语义；当前 `udp.send-recv` 已不再在 executor 内轮询等待 `packet.rx`
- **未实现**
  - `http.*` / `tcp.*` 等通用 action（目前返回 `Failed(not implemented)`）
  - `payload-generator`（sequence/timestamp 等）在 `udp.schedule-send` 中尚未实现
  - 更完善的 metrics 填充与 correlation-id 透传（当前为 best-effort/缺省）

### 2) 下一步里程碑（建议）

- **M1（已完成，对齐事件驱动语义）**：移除 executor 内自旋等待（`udp.send-recv`），统一由 scheduler+状态机处理等待/超时/重试
- **M2（完善发包调度能力）**：补齐 `payload-generator`（sequence/timestamp），并完善 send 相关事件（schedule/tick/completed）以及可观测字段
- **M3（扩展 action 类型）**：落地最小 `http.*` / `tcp.*` action（先保证接口闭环与可观测性，再逐步增强能力）