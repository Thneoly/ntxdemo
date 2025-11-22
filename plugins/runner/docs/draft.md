PMP 启发的 DSL & 调度器架构设计草案

本版本在原有 Load Model + Workflow Model DSL 的基础上，引入 PMP “十五至尊图”（5 大过程组 × 10 类活动/知识领域）的管理思想，将传统有限状态机（FSM）升级为“WBS + Workbook 驱动的事件增强型状态机”。目标是在 wasm32-wasip2 执行环境中，统一规划用户/任务/动作的调度、资源治理以及跨协议组件的协同。

## 1. 背景与目标

- **宏观场景**：在多协议、多组件的仿真平台中定义一组状态与动作；每当动作执行后产生事件，事件可能更新状态并触发后续动作，由此形成对 FSM 的持续演进。
- **PMP 借鉴**：
  - “十五至尊”强调过程组（启动、规划、执行、监控、收尾）与活动域（范围、进度、成本、质量、资源、沟通、风险、采购、干系人、整合）之间的矩阵式协同。
  - 本 DSL 将过程组映射为“脚本流转阶段”，将活动域拆解为可追踪的状态字段，通过 Workbook（状态账本）持续记录。
- **业务落地**：聚焦 wasm32-wasip2 调度器，可通过 HTTP/FTP/SIP 等插件化组件执行动作，满足性能压测、协议模拟与自动化运维的需求。

## 2. PMP 思维与 DSL/FSM 的映射

| PMP 过程组 | DSL 阶段 | 关键产物 | 技术要点 |
| --- | --- | --- | --- |
| 启动 Initiating | Scenario 声明 | `version`、`components`、基础资源 | 决定执行上下文与依赖插件 |
| 规划 Planning | Load & Workflow 设计 | Load Profile、Workflow Steps、WBS | 将任务拆解至 Action 粒度，形成 WBS 树 |
| 执行 Executing | Scheduler 运行 | Wasm 任务、用户实例、Resource Pool | wasm32-wasip2 Runtime + 多协议 Adapter |
| 监控 Monitoring | Workbook 记录 | User/Task State、事件、指标 | Tick 驱动，事件队列，速率与超时监控 |
| 收尾 Closing | 回收与复盘 | Resource GC、日志、度量 | 释放资源、生成报告、同步经验库 |

- **WBS（Work Breakdown Structure）**：将 Scenario→Workflow→Task→Action 形成树状结构，每一层均映射 PMP 的范围/资源/风险等域，保证“定义—执行—监控”的闭环。
- **Workbook**：仿照 PMP 中的 Workbook/Log，用于记录每个 User × Task × Action 的实时状态，字段包含：
  - `state`（idle/running/wait/event/error）
  - `owner`（process group 提供的责任人，比如 Scheduler、Adapter）
  - `risk_flag`、`timeout_at`、`retry_left`（映射风险与进度控制）
  - `evidence`（最近一次事件或上下文快照）

### 当前设计要点速览

- **组件注册表**：`components` 以对象形式记录 `name/version/kind/entry`，并通过 `metadata` 暴露 WIT 接口、能力、传输协议、重试策略等信息，为动作编排与合规审计提供可追踪依赖。
- **资源提供者**：`resources[*].provider.component` 绑定到 `resource-manager` 等 wasm 组件，统一解析 manifest、分配 `pair_pool`、挂载 `file_blob`，并通过 `leasing.ttl/strategy` 治理租约生命周期。
- **Workbook 六大 Section**：`timeline/execution/governance/telemetry/registers/communications` 分别映射 PMP 监控、执行、规划、质量、风险、沟通域，再由 `workbook.expose` 向下兼容旧字段。
- **Graph Workflow**：LangGraph 风格节点支持串行/并行、`controls.exec`（timeout、rate-limit、on-error）和 `transitions`，可将动作与监控节点混排，同时保留 PMP 过程组语义。
- **Load & Monitoring**：`load.profiles` + `scenarios` 驱动用户注入速率，`monitoring` 出口负责超时/错误收敛；`save-as`+`persist` 贯穿执行到收尾的数据沉淀。

## 3. 系统全景架构

```
┌───────────────┐            bundler            ┌───────────────┐
│ Authoring IDE │─────────────────────────────▶ │ Script Bundle │
└───────────────┘                               └──────┬────────┘
                                                       │ (script.yaml / resources / wasm)
                                                ┌──────▼────────┐  Start(res_dir)
                                                │   Host Runtime│
                                                └──────┬────────┘
                                                       │ handover
                                                ┌──────▼────────┐
                                                │   Runner Core │ (parse2wbs + scheduler)
                                                └───┬────┬──────┘
                                                    │    │
  	┌──────────────────────────────┬────────────────┘    └──────────────┐
  	▼                              ▼                                  ▼
Resource Manager            Event Bus / Workbook               Protocol Components
(pair_pool/file_blob)       (state ledger & metrics)           (HTTP/FTP/SIP/...)
```

1. **Authoring / Packaging**：在 IDE 中维护 DSL，并生成包含 `script.yaml`、资源 manifest 与 wasm 组件的 bundle，供 Host 加载。
2. **Host Runtime**：负责 `Start(res_dir)`、挂载 bundle 目录、注入环境变量，然后把 `res_dir` 句柄交给 Runner Core，不参与 DSL 解析。
3. **Runner Core（DSL Parser + Scheduler）**：在 wasm32-wasip2 中运行，执行 `parse2wbs`、构建 WBS DAG，随后调度用户、Tick 和动作。
4. **Resource Manager 组件**：作为 wasm Provider，解析资源 manifest、管理租约（pair_pool/file_blob），并通过 `allocate_pair/release_pair/mount_file` 接口向 Core 提供句柄。
5. **Event Bus / Workbook**：Core 将动作结果、状态变更写入 Workbook Section，并发布到监控/告警；支持速率控制、定时器与 on_error 处理。
6. **Protocol Components**：HTTP、FTP、SIP 等 wasm 模块暴露 `init/run_action/release`，与 Core 通过 WIT 接口通讯，共同完成执行链路。

## 4. DSL 顶层结构（示意）

```yaml
version: "1.0"
components:
  - name: http
    version: "0.2.1"
    kind: wasi-component
    entry: plugins/http/send.wasm
    metadata:
      owner: integration
      wit: wit/http/world.wit
      capabilities: [get, post, delete]
      transport: tcp
  - name: ftp
    version: "0.3.0"
    kind: wasi-component
    entry: plugins/ftp/client.wasm
    metadata:
      owner: integration
      wit: wit/ftp/world.wit
      capabilities: [list, download, upload]
      transport: tcp
  - name: resource-manager
    version: "0.4.0"
    kind: wasi-component
    entry: plugins/resource/manager.wasm
    metadata:
      owner: platform
      wit: wit/resource/world.wit
      capabilities: [allocate_pair, release_pair, mount_file]
      resources:
        provides: [pair_pool, file_blob]
resources:
  ip-port-pool:
    type: pair_pool
    provider:
      component: resource-manager
      calls:
        acquire: allocate_pair
        release: release_pair
      leasing:
        ttl: 120s
        strategy: round-robin
    config:
      manifest: ./resource.yaml
      parser:
        format: csv
        fields: [ip, port]
    capacity:
      size: 1000
      guard: exclusive
  payload:
    type: file_blob
    provider:
      component: resource-manager
      calls:
        mount: mount_file
    config:
      file: ./payloads/asset.json
      checksum: auto
workflows:
  http_asset_refresh:
    wbs-level: L3
    steps:
      - ref: fetch-config
      - ref: pull-asset
      - ref: cleanup
    on-error:
      - action: log.error
      - goto: closing
actions:
  fetch-config:
    component: http
    call: get
    with:
      url: "{{resource.ip}}:{{resource.port}}/cfg"
    save-as: cfg
load:
  profiles:
    ramp:
      start-users: 0
      end-users: 1000
      duration: 60s
  scenarios:
    - name: scenario-http
      workflow: http_asset_refresh
      user-count: 500
      spawn-rate: 100/s
      task-duration: 120s
      exit:
        timeout: 150s
        error-out: fatal
```

### Component Registry Schema

- `components[*].name`：组件或 Adapter 的唯一标识，供 `actions.*.component` 引用。
- `version`：遵循 semver，用于确保调度器加载正确的 wasm 包或服务 Adapter。
- `kind/entry`：指明组件类型（wasi-component/service-adapter/native）及其入口（wasm 路径、RPC endpoint 等）。
- `metadata`：开放式字典，可声明 `owner`、`wit` 接口、`capabilities`、`transport`、`outputs`、`tags`、重试策略等信息，为 PMP 规划/采购/风险域提供可追踪的依赖描述。

Scheduler 在运行前会验证组件版本、加载对应 WIT/wasm，并将 metadata 注入 Workbook / Controls，方便在动作级别做能力检查与审计。

### Resource Provider Schema

- `resources[*].type`：资源族（如 `pair_pool`、`file_blob`、`kv_store`），由对应 Provider 组件宣告支持。
- `provider.component`：绑定到 `components` 中的资源型组件（如 `resource-manager`），通过 `calls.acquire/release/mount` 暴露标准接口。
- `provider.leasing`：定义 TTL、策略（round-robin、least-used）以及是否自动续租，对应 PMP 进度/资源域的分配策略。
- `config`：资源解析配置，比如 `manifest` 文件、parser 格式或 `file` 路径，由 wasm 组件解析后输出结构化资源表。
- `capacity` / `metadata.guard`：限制同一时间的占用，并声明所属 Guard Domain（network/data 等），支撑风险与合规治理。

通过“组件声明 + 资源引用”的方式，脚本可以同时描述资源来源与治理策略。Scheduler 将先实例化 `resource-manager`，由其解析配置、维护租约，再把句柄注入到 Workbook / Actions，确保资源申请与释放全程可追踪。

### 状态、动作、事件的统一抽象
- **State**：来自 Workbook，最小粒度为 User × Task；字段包含阶段（Initiating...Closing）、动作进度、资源占用等。
- **Action**：描述 `component.call` 与参数模版，执行后生成 `event`。
- **Event**：包含 `type`（success/fail/timeout）、`payload`（可选上下文），用于触发状态转移。

## 5. WBS + Workbook 驱动的增强 FSM

1. **WBS 层级**
   - L0 Program：整体压测/仿真项目。
   - L1 Portfolio：协议域（HTTP/FTP/...）。
   - L2 Scenario：单个业务场景（PMP 范围管理）。
   - L3 Workflow：动作编排。
   - L4 Task：针对单用户/单资源对的执行单元。
   - L5 Action：协议插件的原子调用。
2. **状态转移模型**
   - `initiating → planning → executing → monitoring → closing` 作为主态。
   - 在执行态中细分 `waiting_resource / running_action / throttled / blocked`。
   - 事件来自动作结果、资源反馈或系统超时，按照 PMP 活动域决策（如 `risk_flag` 触发应急预案）。
3. **Workbook 实现示例**

```yaml
workbook_entry:
  user_id: U-00042
  task_id: T-http-asset-refresh-7
  state: executing.running_action
  owner: scheduler/http-adapter
  current_action: http.get
  retry_left: 2
  timeout_at: 2025-11-22T10:20:00Z
  scope_trace: [ProgramA, HTTP, Scenario1, WorkflowA, Task7]
  evidence:
    last_event: success
    response_ms: 120
```

Workbook 既是实时监控面板，也是后续复盘与经验库的输入，契合 PMP 中的沟通、质量与风险管理。

为了落实“记录执行过程信息数据”的诉求，全新的 Workbook 被拆分为多个 Section，每个 Section 映射 PMP 的过程组/知识领域：

| Section | 过程组 | 作用 |
| --- | --- | --- |
| `timeline` | Monitoring | 追踪阶段、起止时间与 `transition_trace`，对照进度基线与里程碑 |
| `execution` | Executing | 记录责任人、当前/最近动作、重试预算、`action_log`、证据快照 |
| `governance` | Planning/Stakeholder | 保存 `scope_trace`、RACI、干系人约束，体现 WBS 和责任矩阵 |
| `telemetry` | Quality | 按场景收集 `counters`、`latency_ms` 等性能指标 |
| `registers` | Risk/Quality | 风险/问题登记簿，含 `flag`、条目明细、响应计划 |
| `communications` | Communications | 干系人同步日志与 `last_synced_at`，支撑沟通管理 |

每个 Section 内部都具备 `defaults` 与 `fields` 描述，调度器在运行时按 Section 维度维护 User×Task 的账本，以便差异化持久化或订阅。为保持 DSL 易用性，又引入 `workbook.expose`：

- 通过 `state: timeline.phase`、`scope_trace: governance.scope_trace` 等映射，旧脚本仍可用 `{{workbook.state}}`、`{{workbook.scope_trace}}` 访问常用字段。
- 新脚本可以直接定位到 `{{workbook.telemetry.counters.success}}` 等 Section 化路径，实现多层治理。
- Parser 会根据 `expose` 构建别名表，确保上下文注入与 `user.context`/`update-workbook` 操作均能落到正确 Section。

这种“分层 + Alias” 的 Workbook 既删繁就简，又能把执行轨迹、质量度量、风险管理和沟通记录拆分管理，满足“启动→规划→执行→监控→收尾”的全链路审计与度量诉求。

## 6. wasm32-wasip2 调度器与执行链路

执行链路遵循 `process.txt` 所示的 Host → Core → Protocol 分层：

1. **Host Boot**：仅负责 `Start(res_dir)`，挂载脚本目录、资源文件与组件二进制，随后把 `res_dir` 句柄交给 Runner Core；Host 不再解析 YAML。
2. **Core Parse & Build**：Runner Core 调用 `parse2wbs` 读取 `script.yaml/resource.yaml/config.yaml`，生成 WBS DAG、校验变量与资源依赖，并编织过程组信息。
3. **Protocol Init**：Core 调用 `init_protocol(resource.yaml, res_dir)`，按组件清单加载 wasm/adapter（含 `resource-manager`），执行 WIT 绑定与能力探测。
4. **Scheduler Loop**：
  - Tick ≈ 20 ms（可调），读取 Workbook 中 `online/stop/offline` 用户模型，执行 `spawn`/`stop`。
  - 根据 Load Profile 注入新用户，并维护 `UserState { online, stop, offline, tick }`。
  - 针对在线用户调用 `run_action(task_id, user_id, action_id, params, res)`，其中 `params` 来自 Workbook + Resource Provider。
5. **消息与异常**：协议组件通过 Event Bus 回传执行结果；Core 在 `on_error(error, timeout …)` 中触发 `update-workbook`、`goto closing` 等补偿逻辑，并可做单用户速率控制/定时器调度（呼叫模型库）。
6. **资源治理**：循环结束或节点终止时，Core 经 `release_protocol`／资源组件的 `release_pair/mount_file` 回收租约，保证 Guard Domain 合规。

## 7. 动作编排语义（PMP 版 FSM）

| 功能 | 描述 | PMP 对应 |
| --- | --- | --- |
| `action` | 定义组件调用与参数模版 | 执行过程组 + 范围管理 |
| `with` | 资源/上下文注入（支持 `{{}}` 模版） | 资源、沟通活动域 |
| `save-as` | 将返回值写入 Workbook 上下文 | 整合管理、知识积累 |
| `if/then/else` | 条件分支，驱动状态跃迁 | 风险与沟通活动域 |
| `loop`/`parallel` | 循环或并行动作 | 进度、成本、资源活动域 |
| `on-error`/`on-timeout` | 定义出口与补偿 | 风险、质量、收尾过程组 |
| `rate-limit` | 单用户速率控制 | 进度与资源治理 |

示例片段：

```yaml
- action: http.get
  with:
    url: "{{resource.ip}}:{{resource.port}}/meta"
  rate-limit:
    per-user: 50 req/s
  timeout: 500ms
  save-as: meta
- if: "{{meta.status == 200}}"
  then:
    - action: http.post
      with:
        body: "{{meta.body}}"
      save-as: post_result
  else:
    - goto: monitoring.retry
```

## 8. HTTP Component 宏观场景示例

**目标**：
1. 读取脚本文件，获取 1000 对 `ip:port` 资源。
2. 每个资源对执行 `http-get → http-post → http-delete`。
3. 若超时或错误，统一走 `timeout` 或 `error_out` 出口。

**DSL 片段**：

```yaml
components:
  - name: http
    version: "0.2.1"
    kind: wasi-component
    entry: plugins/http/send.wasm
    metadata:
      capabilities: [get, post, delete]
  - name: resource-manager
    version: "0.4.0"
    kind: wasi-component
    entry: plugins/resource/manager.wasm
    metadata:
      capabilities: [allocate_pair, release_pair, mount_file]
resources:
  ip-port-pool:
    type: pair_pool
    provider:
      component: resource-manager
      calls:
        acquire: allocate_pair
        release: release_pair
      leasing:
        ttl: 120s
        strategy: round-robin
    config:
      manifest: ./resource.yaml
      parser:
        format: csv
        fields: [ip, port]
    capacity:
      size: 1000
      guard: exclusive
workflows:
  http_tri_phase:
    steps:
      - ref: probe-get
      - ref: push-post
      - ref: purge-delete
    timeout: 5s
    on-error:
      - goto: error_out
actions:
  probe-get:
    component: http
    call: get
    with:
      url: "http://{{resource.ip}}:{{resource.port}}/asset"
    save-as: asset
  push-post:
    component: http
    call: post
    with:
      url: "http://{{resource.ip}}:{{resource.port}}/asset"
      body: "{{asset.body}}"
    save-as: result
  purge-delete:
    component: http
    call: delete
    with:
      url: "http://{{resource.ip}}:{{resource.port}}/asset"
load:
  scenarios:
    - name: http-bulk
      workflow: http_tri_phase
      user-count: 1000
      spawn-rate: 200/s
      user:
        inherit:
          - ip-port-pool
      exits:
        timeout: timeout_out
        error: error_out
```

**执行要点**：
- Scheduler 先实例化 `resource-manager`，通过 `acquire/release` API 借出 `ip:port` 并记录在 Workbook `scope_trace`。
- `probe-get` 成功后将响应写入 `asset`，供下游动作引用。
- 超过 Workflow 的 5s 或 Action Timeout 则写入 `timeout_out`，并在 Workbook 中标记 `state=closing.timeout`。

## 9. 变量模型与传递机制

为避免“变量散落”或作用域混乱，需要将 DSL 内的状态数据划分为若干层次，并定义一个统一的传递通道。整体变量体系如下：

1. **Global/Scenario 级变量**
  - 来源：`components`、`resources`、`load.scenarios[*].user.context`、`monitoring` 等顶层配置。
  - 用途：描述静态依赖（插件名、WIT 接口、IP 资源池），以及每个 Scenario 的基线上下文（如 `scope_trace`）。
  - 访问：通过 `{{global.xxx}}` 或 `{{resource.ip}}` 模版注入到动作参数中。

2. **Workbook / User-Task Context**
  - 在 `workbook.defaults/fields` 中声明字段，运行时为每个 `User × Task` 维度维护一份状态。
  - 典型字段：`state`、`owner`、`timeout_at`、`evidence`，以及业务自定义字段（如 `last_http_status`）。
  - 访问：`{{workbook.state}}`、`{{workbook.scope_trace}}`；更新通过 `update-workbook` 指令或动作返回值自动写入。

3. **Action Execution Context**
  - 每次节点执行动作时，会生成一个局部上下文：
    - `inputs`：来自 `with` 里渲染后的参数。
    - `env`：Scheduler 注入的调度变量（tick、user_id、task_id、phase 等）。
    - `locals`：动作实现内部产生的临时数据，仅在 wasm 组件内部可见。
  - 超时、限流、重试等控制逻辑也挂在该层（见 `controls.exec`).

4. **Return / Save-As 变量**
  - `save-as` 负责将动作输出映射到命名变量，可配置写入位置：
    ```yaml
    save-as:
     var: asset
     scope: workbook   # or user-local / task-local / global
     persist:
      type: file
      dir: ./runtime/artifacts/assets
      filename: "{{workbook.user_id}}-asset.json"
    ```
  - 当 `scope=workbook` 时，`asset` 自动成为 `{{asset}}` 或 `{{workbook.asset}}` 的模板变量，供后续动作引用。
  - `persist` 可选，用于将数据落盘/推送到对象存储等。

5. **节点级依赖传递**
  - Workflow 节点在 `controls.flow` 中声明 `transitions`。执行顺序由节点控制，而数据依赖通过 Save-As+模版实现：
    - `init_probe` 写入 `asset`。
    - `post_push` 的 `with.body: "{{asset.body}}"` 即可引用上一动作的返回值。
  - 对于并行分支，变量遵循“写入者负责取名”的约定：若两个分支都写 `asset`，同名变量会被最后写入者覆盖，可通过命名空间（`asset_probe`, `asset_cleanup`）避免冲突。

6. **全局变量一致性**
  - 所有变量在解析阶段会构建一个变量图（Variable Graph），验证依赖是否合法（例如在引用 `asset` 前是否有某个节点负责生成）。
  - 若存在循环依赖或未定义变量，Parser 会在部署前报错。

通过上述分层，变量在 DSL 中的生命周期清晰：全局 → Workbook → 节点 Action → Save-As → 下游节点。调度器只需根据节点执行顺序和 `save-as` 规则，就能完成从一个动作到下一个动作的上下文传递，同时保持可观测性与可追踪性。

## 10. Timeout / Error 治理

- **统一出口**：所有 Workflow 必须声明 `timeout_out` 与 `error_out`，由 Scheduler 连接至 PMP 收尾过程（记录、复盘、资源回收）。
- **多级 SLA**：
  1. Action 级超时 —— 由组件 Adapter 直接上报。
  2. Task 级超时 —— Workbook 维护 `start_at/timeout_at`，Tick 检查。
  3. Scenario 级超时 —— Load Profile 可设置 `max-duration`，触发批量终止。
- **补偿策略**：在 `on-error` 中可定义 `goto monitoring.retry` 或 `goto closing.rollback`，确保质量与风险域被覆盖。

## 11. 扩展与落地计划

1. **完成 DSL Schema 定义**：使用 JSON Schema/WIT 描述 DSL 结构，方便 IDE 校验。
2. **Wasm 调度器增强**：在 `plugins/runner` 中实现 Workbook KV 存储、事件总线以及 `rate-limit` CPS 调控。
3. **可观测性**：输出 Prometheus 指标（过程组维度）与 Workbook Change Stream，支撑沟通与干系人管理。
4. **知识沉淀**：自动把 Workbook 结果归档至“经验库”，作为下一次 Scenario 的基线（契合 PMP 持续改进）。

通过把 PMP 的过程治理能力与 DSL/FSM 的执行能力结合，本方案既能支持复杂协议的并发调度，又能提供全过程可审计的运行视图，为 wasm32-wasip2 平台上的调度器与动作编排奠定统一的架构基础。