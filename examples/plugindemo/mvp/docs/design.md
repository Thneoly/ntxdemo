# MVP 交互设计说明

本文来自原有手绘流程图的文字化版本，用于对齐 Host、Core、Protocol 三个模块之间的职责与调用关系。

## 1. 模块划分与核心职责

| 模块 | 职责概述 | 对外接口 |
| --- | --- | --- |
| Host | 启动组件，准备配置目录，并将其传递给 Core；负责生命周期的起停与资源目录管理。 | `start(res_dir: string) -> void`（唯一暴露给主站的接口） |
| Core | 解析 YAML/DSL，构建 WBS（Workbook Structure）树；调度用户模型与 action；封装公共运行时（日志、定时器、网络、呼叫模型等）；调用业务 Protocol 的 `init/run/release`。 | `init_protocol(...)`、`run_protocol(...)`、`release_protocol(...)`；同时向 Protocol 注入 runtime lib（logger/timer/socket/send/rcv/call-model）。 |
| Protocol | 面向具体业务协议（HTTP/FTP…），按照配置执行 action；维护自身状态（`user_id`, `task_id`, `action_id` 等），并在完成或出错时回调 Core。 | `init(res_yaml, res/config.yaml)`、`run_action(task_id, user_id, action_id, params, res)`、`release()`、`on_error(...)` |

## 2. 配置与资源打包

Host 在调用 `start(res_dir)` 前，需要准备如下目录结构并整体传入 Core：

```
res_dir/
├── script.yaml        # workflow + actions
├── resource.yaml      # ip_list, port_list 等资源清单
├── config.yaml        # 全局配置
└── resource/
    ├── config.yaml    # component 局部配置
    └── http.cap       # 协议/抓包等额外文件
```

核心约定：Host 不解析内容，只负责把目录交给 Core； Core 负责进一步读取并解析。

## 3. 生命周期与流程

1. **Host → Core：Start**  
   Host 调用 `start(res_dir)`，并将资源目录的绝对路径传入 Core。

2. **Core：parse2wbs**  
   Core 读取 `script.yaml`，将 Action DSL 解析为 WBS Tree；并读取 `resource.yaml`、`config.yaml`、`resource/` 下的私有配置。

   > `script.yaml` 是唯一的 Action 定义来源：Host 负责把该文件随目录下发，Core 负责解析/校验，并映射到内部的 WBS 节点。Protocol 不直接读取 `script.yaml`，而是通过 Core/PF 注入的 `action_ctx` 获得所需字段（payload、期望速率、依赖等）。这样可以确保所有 action 的生命周期、依赖拓扑都由 Core Scheduler 统一管理。

3. **Core：init_protocol**  
   对每个 Protocol 组件调用 `init(res.yaml, res/config.yaml)`；此时也会注入 runtime lib handler（logger/timer/socket/send/rcv/call-model）。

4. **Core：run_protocol**  
   - Core 维护 `UserState`：在线 (`online`)、待停止 (`stop`)、离线 (`offline`) 用户数量 + 调度 tick（如 `tick: 20/300`）。  
   - 调度器遍历 WBS Tree，将 action 映射到具体 Protocol 并触发 `run_action(task_id, user_id, action_id, params, res)`。
   - Protocol 内部 `do_action` 可通过 Core 提供的 API（例如 `send_data(bytes, tick, times)`）发起报文收发。

5. **状态回传与控制**  
   - Protocol 完成 action 后，调用 Core runtime 的 `on_finish` 或 `notify_state(user_id, task_id, action_id, state)`。  
   - Core 根据配置决定用户模型（在线用户数限制、单用户速率控制）以及是否将用户状态迁移至 stop/offline。

6. **异常处理**  
   Protocol 在检测到错误或超时时调用 `on_error(error, timeout, …)`，Core 根据策略发起重试或回退。

7. **Core：release_protocol**  
   所有调度结束后，Core 调用 `release()`，释放 Protocol 占用的资源；Host 在确认结束后销毁组件进程。

## 4. Core 运行时能力设计（对 Protocol 暴露）

| 能力 | 说明 | 建议接口 |
| --- | --- | --- |
| Logger | 统一日志入口，附带 `task_id/user_id/action_id` 标签。 | `log(level, message, ctx)` |
| Timer | 设定 tick、延迟或周期任务，驱动用户模型。 | `schedule(delay_ms, callback_id)`、`cancel(timer_id)` |
| Socket | 管理 TCP/UDP 会话，限制端口范围。 | `open_socket(params) -> handle`、`close(handle)` |
| Send/Recv | 面向协议的报文发送、接收缓存，支持速率限制。 | `send_data(handle, bytes, tick, repeat)`、`recv_data(handle, max_bytes)` |
| Call Model | 调用“呼叫模型算法库”，根据用户模型输出在线/离线/停止数量或节奏。 | `next_user_state()`、`apply_rate_limit(user_id, rate_cfg)` |
| Error Channel | 收集 Protocol 的 `on_error`、`on_finish`，并投递到调度器或 progress bus。 | `notify(event)` |

## 5. Protocol 组件接口语义

1. `init(res_yaml, res_config_yaml)`  
   - 解析自身所需的 IP、端口、鉴权数据。  
   - 注册所需的 Core runtime 能力（logger/timer/socket 等）。

2. `run_action(task_id, user_id, action_id, params, res)`  
   - 在 Core 的调度下按 `online_user_list` 遍历用户。  
   - `do_action` 内部可多次调用 `send_data`/`recv_data`，并根据 tick/times 控制速率。  
   - 更新自身状态机（`user_id`, `task_id`, `action_id`, 当前进度/错误）。

3. `release()`  
   - 释放 socket、定时器等资源。  
   - 报告最后的统计（成功数、错误数、平均耗时）。

4. `on_error(error, timeout, …)`（回调 Core）  
   - Core 根据错误类型决定：重试、停止该用户、或回退整个任务。

### 5.1 ProtocolFrame 抽象

为避免每个业务协议重复实现调度胶水代码，我们在 Protocol 与 Core 之间新增 `ProtocolFrame`（简称 PF）层，负责以下通用能力：

| 能力 | 说明 | 公共 API（示例） |
| --- | --- | --- |
| Task/Action 管理 | 维护 `task_id → action队列 → user实例` 的映射，提供状态查询/更新。 | `pf.register_task(task_meta)`、`pf.claim_action(task_id, user_id)`、`pf.complete_action(task_id, action_id, result)` |
| Context 持久化 | 为每个 Protocol 实例保存 `user_id/task_id/action_id` 及自定义上下文，支持快照与恢复。 | `pf.store_ctx(task_id, user_id, ctx)`、`pf.load_ctx(task_id, user_id)` |
| 状态同步 | 将 Protocol 状态增量写入 progress bus，或通过 Core runtime 的 `notify_state`。 | `pf.emit_progress(scope, phase, progress, fields)` |
| 节流与速率限制 | 复用 Core 提供的 `send_data/recv_data`，额外记录 per-user/per-action 的速率配置。 | `pf.guard_send(user_id, rate_cfg, send_closure)` |
| 错误与重试 | 统一封装 `on_error`、重试策略和告警打点。 | `pf.fail(task_id, user_id, action_id, error)`、`pf.schedule_retry(task_id, action_id, backoff)` |

PF 的位置关系如下：

```
Core 调度器 ──调用──> ProtocolFrame (PF) ──委托──> 具体 Protocol 实现
```

工作流程：
1. **初始化**：Core 实例化 PF，并注入 runtime 能力（logger/timer/socket 等）。Protocol 在 `init` 中优先向 PF 注册任务、加载上下文。  
2. **执行**：调度器下发 action 时，可先由 PF 进行合法性校验、并发度控制，再调用具体 Protocol 的 `do_action`。  
3. **状态维护**：Protocol 将临时状态传给 PF，PF 负责落盘/同步；完成后由 PF 调用 Core 的 `notify_state` 与 `progress.wit`。  
4. **收尾**：`release()` 期间，PF 统一清理由其托管的资源（上下文缓存、速率控制器等）。

引入 PF 后，业务 Protocol 仅关注协议-specific 的报文逻辑，Core 也只需与 PF 对齐一组稳定接口，降低多协议并行演进的成本。

## 6. 调度与控制要点

- **在线用户控制**：Core 根据呼叫模型在每个 tick 调整 `online/stop/offline` 数量，并把新的用户集合传给 Protocol。  
- **单用户速率控制**：Core 下发 `rate_profile`（bytes/tick/times），Protocol 必须遵守；Core 通过 send/recv API 内置节流。  
- **定时器/模型触发**：呼叫模型可以由 Core 定期触发，也可以暴露 `timer` 给 Protocol，在 `do_action` 中注册回调。

## 7. 后续对齐项

- 明确 `WBS Tree` 的节点属性（Action 类型、依赖关系、权重）。
- 定义 `notification` 通道，将 Protocol 的完成事件与 Progress Bus 对接。  
- 对齐 `resource.yaml` / `config.yaml` 的字段 schema，避免 Host/Core/Protocol 对解析口径不同。
- 细化 Core runtime API 的安全策略（socket 访问范围、日志敏感字段屏蔽等）。

该文档后续如有流程变更（新增接口或能力）请在此基础上继续补充，以保持 Host/Core/Protocol 交互一致。

## 8. 模块接口设计一览

当前实现由四个主要模块组成，约束在单一业务组件（`Core + ProtocolFrame + Protocol`）中，通过 WAC 组合后再由 Host 加载。下表概述每个模块对外暴露的接口层次、触发方和典型责任。

| 模块 | 载体 | 对外接口 | 触发方 / 说明 |
| --- | --- | --- | --- |
| Host | 独立可执行或守护进程 | `start(res_dir: string) -> void`；（后续可扩展 `stop()`、`health()`） | 上层控制面调用，负责准备资源目录并加载目标 component。 |
| Core（Scheduler + Libs） | 组件内部的主实例 | **导出给 Host**：`run(args)`；**导出给 ProtocolFrame**：`logger`, `timer`, `socket`, `event`, `call_model`, `progress`；**导入自 Protocol/PF**：`init`, `run`, `release`, `notify` | Host 进入 Core 的 `run`，Core 在内部初始化 Scheduler、Libs，并把 runtime capability 传给 PF/Protocol。 |
| ProtocolFrame (PF) | Core 内部子实例 | **对 Core 暴露**：`pf.init(ctx)`, `pf.dispatch(action_ctx)`, `pf.release()`；**对 Protocol 暴露**：`pf.borrow_runtime()`, `pf.task_store`, `pf.rate_guard`, `pf.progress_sink` | PF 作为桥梁，将 Core 调度、资源复用逻辑与具体 Protocol 分离。 |
| Protocol | 业务协议实现（HTTP/FTP/自定义） | `protocol.init(init_ctx)`, `protocol.run_action(action_ctx)`, `protocol.release()`, `protocol.on_error(error_ctx)` | PF 调用 Protocol 完成具体 action；Protocol 通过 PF/ Core runtime 获取资源、上报状态。 |

### 8.1 Host ↔ Core 接口

- **Host 导入**：`world progress-runner`（或扩展 world）中的 `run` / `progress` / `events`。Host 只需调用 `run`，并传递 `res_dir`、`scenario` 等参数。  
- **Core 导出**：可选提供 `control` 接口以便 Host 查询运行状态（未来扩展）。  
- **配置传递**：Host 将 `res_dir` 作为参数传入 Core；Core 在 `run` 开头读取目录。

### 8.2 Core ↔ ProtocolFrame 接口

- Core **提供** runtime traits：
   - `Logger`: `fn log(level, message, ctx)`
   - `Timer`: `fn schedule(delay_ms, id)` / `fn cancel(id)`
   - `Socket`: `fn open(params) -> handle`, `fn send(handle, bytes)`、`fn recv(handle)`
   - `EventBus`: `fn notify(event)`（内部接入 progress.wit）
   - `CallModel`: `fn next_state(tick) -> UserState`

- PF **调用 Core**：
   - `core.register_pf(pf_handle)` / `core.unregister_pf()`
   - `core.dispatch(action_ctx)`（由 Scheduler 驱动，返回 `DispatchOutcome`）
   - `core.progress(update)`、`core.raise(error)`（透传到 Host/监控）

### 8.3 ProtocolFrame ↔ Protocol 接口

- PF 暴露的接口：
   - `pf.init(protocol_handle, init_ctx)`：写入 task/action metadata，装配 runtime。
   - `pf.next_action(protocol_handle) -> Option<ActionCtx>`：对 Protocol 提供安全的 action 借阅。
   - `pf.commit(protocol_handle, action_ctx, result)`：提交执行结果，自动上报进度。
   - `pf.fail(protocol_handle, error_ctx)`：统一错误路径，触发重试或降级。

- Protocol 需要实现：
   - `protocol.init(init_ctx)`：协议内部初始化。
   - `protocol.run_action(action_ctx)`：在 PF 提供的上下文中执行。
   - `protocol.release()`：释放资源并回收上下文。
   - `protocol.on_error(error_ctx)`：可选，处理 PF 注入的异常通知。

### 8.4 模块组合与 WAC

在代码层面，Core + PF + Protocol 被组织成一个 component，导出的 world 示例：

```wit
package ntx:runner;

world workload-component {
      import host: interface { start: func(res_dir: string); } // Host 能力（可选）
      export runner: interface { run: func(res_dir: string); }
      export progress: runner:progress/progress-runner.events;
}
```

构建时：

1. Core (含 Scheduler + Libs) 作为主组件。  
2. PF、Protocol 作为依赖组件，通过 `wac` 或 component linking 与 Core 合并。  
3. Host 加载合成后的 `.wasm`，调用 `runner.run(res_dir)` 启动单个业务实例。

该接口章节会随着具体 `wac` 组合方式（例如多 world 导出、控制面接口）进一步细化，当前版本可指导模块并行开发与集成。

### 8.5 动态任务扩展（Protocol 增加新 Task）

有些协议需要在运行过程中根据业务反馈追加 task（例如临时探测、二次拨测）。为了保证新增任务仍受 Scheduler 控制，我们约定以下流程：

1. **Protocol 发起请求**：Protocol 在 `run_action` 中调用 PF 提供的 `pf.request_task(new_task_meta)`。元数据包含 `task_template`（引用 `script.yaml` 中已有 action 模板）或自定义 action 描述。  
2. **PF 校验与封装**：PF 根据当前调度上下文补全缺省字段（所属 workflow、优先级、用户集合），并执行静态校验（避免突破在线用户上限、速率限制）。  
3. **Core Scheduler 接入**：PF 通过 `core.scheduler.enqueue(task_descriptor)` API 将新任务提交回 Core。Scheduler 会：
   - 重新映射到 WBS：若引用现有模板，则使用模板节点生成新的子节点；若为自定义 action，则生成临时节点并记录来源。  
   - 重新分配调度槽位（tick/用户模型），确保不会与现有 task 冲突。  
   - 将新的 `action_ctx` 派发给相应 Protocol。
4. **反馈机制**：Scheduler 将新增 task 的 `task_id` / 状态通过 PF 回传给 Protocol，便于其跟踪完成度。
5. **可观测性**：所有动态任务在 progress bus 中增加 `origin = dynamic` 标签，方便 Host/监控区分。

通过这套机制，Protocol 可以在不绕过 Core 控制的前提下扩展任务集合，而 Scheduler 仍掌握最终调度权，确保资源配额与用户模型的一致性。