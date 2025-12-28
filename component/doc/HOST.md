
# Host 最终方案（破坏性终局版）：run() 内 pull RX ring（handle/offset）

本文档只保留 **最终落地形态**（破坏性，不考虑兼容）：

1) host 只调用一次 `scheduler-component.run(config-dir)` 拉起 guest scheduler 的主循环（长期运行）。
2) 运行期 host **不再**通过任何 guest 导出函数注入 RX（包括 `packet-ingest.notify-rx`）。
3) 收包数据通过 **guest → host import `rx-ring`** 在 `run()` 内部主动拉取，并转译为 `packet.rx` 事件驱动状态机。

> 术语：
> - “host” 指 `ntx` 宿主进程（root crate `src/`）。
> - “guest” 指 wasm32-wasip2 component（WAC 组装后的 composed scheduler）。

## 最终 WAC/WIT 启动契约

对外导出（composed scheduler）：

- `ntx:scenario-scheduler/scheduler-component@0.1.0`
	- `run(config-dir: string) -> result<_, string>`：拉起 guest scheduler 主循环（长期运行、不应依赖返回）。

对内导入（guest → host）：

- `ntx:host/rx-ring@0.1.0`：在 guest `run()` 内拉取 RX 批次（handle/offset ABI）。

> ✅ 现状提示：仓库当前仍 export 了 `packet-ingest.notify-rx`（WIT/WAC 均可见）。
> 终局版落地时必须删除对应 export + host 调用链，避免误用回退。

## `ntx:host/rx-ring@0.1.0`（handle/offset ABI）

目标：**避免每批次把 desc/payload 整块复制到 guest**。host 维护一个 bounded 的 batch 队列，guest 获取一个轻量 handle，再按需读取 slice，最后显式释放。

### 类型
- `type batch-handle = u64`：host 对一个 batch 的逻辑引用（非指针）。
	- 语义补充（写死）：`handle` 在 host 内部必须绑定到某个具体的 backing store（region/slab/pool）；`read-*` 的 `off/len` 统一是**相对该 handle 对应的 desc/payload 起点**的偏移。
	- 约束（写死）：在 `release()`（或 lease 过期回收）之前，host **不得复用覆盖**该 handle 对应的 backing store。
- `record rx-batch`：
	- `handle: batch-handle`
	- `desc-len: u32`（bytes）
	- `payload-len: u32`（bytes）
	- `seq: u64`（递增序列，用于观测）
		- 语义（写死）：`seq` 为 **batch 级单调递增** 的编号（每产生一个可见 batch，`seq += 1`）。
		- 允许跳号：当发生丢弃/合并/回收导致 batch 不可见时允许跳号；用于观测“系统是否在丢”。

### API（最小集合）

- `poll-rx(max-desc: u32, max-payload: u32) -> option<rx-batch>`
	- 若队列为空：返回 `none`。
	- **规范语义（写死）**：host **不得**返回截断视图。若某 batch 超出上限（`desc-len > max-desc` 或 `payload-len > max-payload`），host 必须在入队前拆分或选择一个满足上限的 batch；否则返回 `none` 并记录指标。

- `wait-rx(max-desc: u32, max-payload: u32, timeout-ms: u32) -> option<rx-batch>`（可选）
	- 在超时窗口内等待一个 batch；实现可为阻塞等待或 poll+短睡。
	- 语义（写死）：
		- `timeout-ms` 到期必须返回 `none`。
		- host 在 **enqueue 新 batch** 时必须唤醒正在等待的 `wait-rx`（例如 condvar notify）。
		- host 进入 shutdown 时也必须唤醒 `wait-rx`，使其尽快返回 `none`（避免 run-loop 卡死在一次 import 调用上）。

- `read-desc(handle: batch-handle, off: u32, len: u32) -> result<list<u8>, string>`
- `read-payload(handle: batch-handle, off: u32, len: u32) -> result<list<u8>, string>`
	- 用于按需小块读取，典型读取粒度为：control block + 若干 descriptors + 对应 payload slice。

- `release(handle: batch-handle) -> result<_, string>`
	- guest 必须释放 handle，host 才能回收/复用底层 buffer。

### 生命周期与错误语义（写死）

1) `handle` 在 `release()` 前有效；release 后任何 `read-*` 必须返回 `err("invalid handle")`。
2) host 必须 bounds check：`off+len <= desc-len/payload-len`，否则 `err("out of bounds")`。
3) host 必须校验 handle 状态：不存在/已释放/已过期 → `err("invalid handle")`。
4) host 必须实现 `lease timeout` 防泄漏：建议默认 **5000ms**。
	- 超时允许强制回收，并记录指标（例如 `rx_ring.lease_expired_total`）。
	- 约束（写死）：超时回收后
		- 后续 `read-*` 必须稳定返回 `err("invalid handle")`
		- host 可以复用底层 backing store，但**不得让旧 handle 误读新数据**：建议实现 `handle = (slot_id, generation)`（或等价机制），并在复用时递增 generation；旧 handle 必须被识别为 `invalid handle`。
5) `desc/payload` 的内存布局复用 `src/wasm_engine/shared_mem.rs`（control block + desc ring + payload ring；desc 32 bytes）。

## Host 侧最终运行模型（只保留两条链路）

### 1) Engine owner / guest run（独占执行体）

- host 启动后创建一个 **engine owner**（专用线程或 Tokio `spawn_blocking` 的单线程 owner）。
- engine owner 仅负责：实例化 composed scheduler，并调用一次 `scheduler-component.run(config-dir)` 进入长期运行。
- 关键约束：运行期 engine owner 不再对同一实例执行任何“额外导出调用”。

### 2) NIC RX → 入队 batch（bounded + backpressure）

- NIC RX 任务持续收包。
- 将包写入 shared-mem 格式的 desc/payload buffer（复用 `shared_mem` 约定）。
- 将该 batch 放入 **bounded 队列**（供 `rx-ring.poll-rx/wait-rx` 取走）。

- 队列满时策略（写死，默认）：**drop newest（丢当前入队的 batch/包）**，并记录可观测指标。
	- 最小指标建议：
		- `rx_ring.enqueue_drop_total`
		- `rx_ring.queue_depth`
		- `rx_ring.inflight_handles`
		- `rx_ring.bytes_in_queue`
	- 说明：不推荐用“阻塞 NIC RX 生产者”做背压（尤其是在单核/高负载时），这会把 host 数据面拖死，和“系统不许卡死”的验收目标冲突；如需背压，请在更上游（例如接收队列/应用层）提供可控限速。

## Guest 侧最终消费模型（在 run-loop 内）

guest `run()` 主循环每次迭代：

1) `wait-rx/poll-rx` 获取 `rx-batch(handle, ...)`。
2) 通过 `read-desc/read-payload` 按需读取并 decode。
3) decode 成功后发布 `packet.rx` 事件（eventbus publish）。
4) 无论成功/失败，都必须 `release(handle)`（避免 lease timeout 回收）。

## 验收标准（必须可验证）

1) host 进程启动后只调用一次 `scheduler-component.run()`。
2) 运行期 host 不存在任何 `notify-rx`/“导出注入 RX”路径（包括 eventbus 导出注入等变体）。
3) 压测下系统不因锁/重入卡死；只允许出现“背压/丢弃”的可观测行为。
4) `lease timeout` 生效：即便 guest 故障不 release，host 也能回收并打点。

## Host 侧运行方式整改（避免 `run()` 常驻导致的死锁）：Tokio 异步化方案

### 背景：现有多线程模型的典型死锁触发点

当前 `src/main.rs` 会在专用线程中执行 `EngineManager::global().lock()` 并调用 `mgr.run(...)`，而 `run()` 是 guest 内部 `loop {}` 常驻，不会退出。

这会引入一个非常危险的结构：

1) **`EngineManager` 的 Mutex 被 `run()` 线程长期持有**（因为 lock 在 `run()` 返回前不会释放）。
2) 运行期只要 host 其他线程/任务需要通过 `EngineManager` 做任何事情（哪怕是 enqueue RX batch、未来的控制面调用等），就会永久阻塞在同一个 Mutex 上。
3) 更坏的情况是：如果阻塞发生在持锁/资源持有的上下文里（例如某些共享资源、日志/指标回调、或未来引入的跨 wasm call 锁），会形成 **锁序反转** 或 **等待环**，从而表现为死锁。

> 关键结论：问题不在于“线程多”，而在于“把长期运行的 `run()` 放在持有全局锁的临界区内”。

### 目标约束（写死，作为 guardrail）

- host **只能调用一次** `scheduler-component.run(config-dir)`。
- `run()` 必须运行在一个独占执行体中，但该执行体 **不得**长期持有 `EngineManager` 等全局锁。
- 运行期任何 host 侧锁（`std::sync`/`parking_lot`/`tokio`）都 **不允许跨 wasm call 持有**（包括 `run()` 以及任何未来的导出调用）。

### 最终方案：Tokio-native + Wasmtime component async

这一节只描述最终形态：host 全面 Tokio 化，并把 guest `run()` 变成 **可被 Tokio 调度的 async wasm 调用**。

核心思想：

- host 使用 Tokio 统一调度：NIC RX/TX、控制面、指标、wasm 侧等待（`wait-rx`）都在同一个 async 体系里。
- Wasmtime 启用 component async 支持：`scheduler-component.run()` 以 async 方式 drive，避免“run() 常驻 = 永久占用一个线程”的结构性问题。
- 仍然坚持 **单一所有权/串行化访问 Wasm 实例**：即使是 async，也只允许一个 task 驱动同一个 `Store/Instance`；其他任务只能通过消息/共享状态与其协作。

> 目标不是“并发调用 wasm”，而是让 wasm 的阻塞点（如 `wait-rx(timeout-ms)`）能让出执行权，从而把系统推进交还给 Tokio。

#### 具体实现建议（最小侵入、兼容当前结构）

1) **去除“在持有 `EngineManager` 锁时调用 `run()`”**：
	- 在启动阶段：`let engine = ComponentEngine::new(cfg)` 完成后，把它 move 到 engine owner。
	- `EngineManager` 只作为“注册表/启动期构造器”，启动完成后不再要求运行期持锁访问。

2) **Tokio runtime 统一调度**（避免“主线程 scheduler loop + guest thread”这种容易引入锁/生命周期交错的结构）：
	- `#[tokio::main(flavor = "multi_thread")]`
	- NIC RX/TX、指标、控制面都使用 `tokio::spawn`。

3) **打开 Wasmtime async 支持**（终局前置条件）：
	- host 侧使用支持 async 的 Engine/Linker/Store 配置（以本仓库现用 Wasmtime 版本/API 为准）。
	- `scheduler-component.run()` 以 async 方式执行，允许在 import（例如 `wait-rx`）内部发生 await/yield。
	- 对应地，host 的 WIT import 实现应尽量使用 `tokio::sync`（例如 `Notify`/`Semaphore`/`mpsc`）来实现“等待 + 唤醒 + timeout”。

4) **shutdown 语义**（避免 `wait-rx` 卡死）：
	- host 触发 shutdown 时：先让 NIC RX 停止入队，然后显式调用 `rx_ring.shutdown()`（若已有）或设置共享 flag 并 `notify`，保证 `wait-rx` 尽快返回 `none`。
	- engine owner 任务可选择：继续运行（直到进程退出），或在 guest 支持退出信号后再做可控停止。

5) **锁使用纪律（强制）**：
	- `EngineManager`/`ComponentEngine` 相关可变访问，统一发生在 engine owner 任务内。
	- 跨 wasm call（尤其是 `run()`）前必须确保：没有持有任何会被其他任务也需要的锁。

### 为什么“Tokio-native + Wasmtime async”是必要的（而不是临时线程方案）

- 本问题的根因是：`run()` 常驻 + 同步执行 + 非重入，使得“任何需要与 wasm 交互的路径”都容易被结构性阻塞放大。
- 仅仅把 `run()` 放到专用线程，最多是把卡死从“抢 Mutex”变成“跨线程协作困难、退出不可控、等待/唤醒不在同一调度体系”。
- 开启 Wasmtime async 后，`wait-rx(timeout-ms)` 这类等待点可以在同一 Tokio runtime 下自然地 yield/timeout/shutdown，host 的推进逻辑更清晰，观测也更统一。

### 最小验收（与本文档验收标准对齐）

- host 启动后：只发生一次 `scheduler-component.run()` 调用。
- 运行期：`EngineManager` 不会因为 `run()` 常驻而长期被锁住（可通过打点/trace 或 debug assert 验证）。
- 压测下：允许 drop/backpressure，但不允许出现“线程全部卡死/任务不再推进”。

### 前置条件与风险提示（写在这里，避免误用）

- 需要 Wasmtime component async 能力与本仓库的 WIT binding 生成方式兼容；若升级 Wasmtime 版本，务必同时校验 `component::bindgen` 生成的 async host trait/签名。
- 即使是 async，也必须坚持“单 owner 驱动一个 `Store/Instance`”；不要试图并发地从多个 task 调用同一个 instance。
- 对 `rx-ring` 的实现：`wait-rx` 必须是 **Tokio 原语驱动的异步等待**（例如 `Notify`/`Semaphore`/`mpsc` + `timeout`）；如果内部仍用条件变量/阻塞锁等待，等价于把阻塞重新塞回 Tokio worker，最终仍会出现“系统推进停滞”。

## 整改计划 & TODO

本节是从当前仓库状态推进到本文档“终局版契约”的落地 checklist（按依赖顺序）。

### 0. Host 运行模型异步化（Tokio / Engine Owner）

- [x] 将 host 主入口改为 Tokio runtime（`#[tokio::main]`），统一调度 NIC RX/TX 与控制面任务
- [x] 启用 Wasmtime component async（以本仓库 Wasmtime 版本/API 为准，必要时升级并同步更新 bindgen 生成代码）
- [x] 引入 **Engine Owner actor**（单一所有权执行体）：独占持有同一个 `ComponentEngine/Store`，以 **async** 方式驱动 `scheduler-component.run()`（只调用一次）
- [x] 严禁在持有 `EngineManager`（或其他全局锁）时调用 `run()`：启动期构造完成后 move ownership 给 owner
- [x] 将 `rx-ring.wait-rx` 的等待/唤醒/超时改为 Tokio 原语实现（避免阻塞锁/condvar 重新把阻塞塞回 Tokio 线程）
- [ ] 为 host 增加 guardrail：任何锁都不允许跨 wasm call 持有（对 `run()`/其他 wasm 调用路径增加 debug 断言/注释约束 + code review checklist）
- [x] shutdown 验收：触发关机时必须唤醒 `wait-rx` 并保证任务可退出/可收敛（避免 run-loop 无法响应 shutdown）
- [ ] 增加最小观测：记录 owner 健康状态（心跳/卡顿告警/队列深度等），用于定位“推进停滞”

### A. 接口契约（WIT/WAC）

- [x] 新增 WIT：`ntx:host/rx-ring@0.1.0`（handle/offset ABI）
	- [x] 定义 `batch-handle`, `rx-batch`
	- [x] 定义 `poll-rx / wait-rx / read-desc / read-payload / release`
	- [x] 错误字符串写死：`invalid handle` / `out of bounds`
	- [x] 写死语义：`wait-rx(timeout-ms)` 超时必须返回 `none`；host shutdown 必须唤醒 `wait-rx` 使其尽快返回 `none`
	- [x] 写死语义：句柄复用规则（release/过期后 backing store 复用）必须通过 generation 隔离（避免旧 handle 误读新数据）

- [x] 修改 scheduler `world.wit`
	- [x] `import ntx:host/rx-ring@0.1.0`
	- [x] 删除 `export ntx:scenario-scheduler/packet-ingest@0.1.0`（`notify-rx`）

- [x] 修改 `scheduler-composition.wac`
	- [x] composed scheduler **不再 export** `packet-ingest`
	- [x] 为 scheduler world 接入 host `rx-ring` 实现（import wiring）

- [ ] 验收
	- [x] composed 产物对外仅 export：`scheduler-component.run`
	- [x] composed 产物对内 import：`rx-ring.*`（以及其他必要 imports）

### B. Guest（scheduler component）

- [x] `run()` 主循环内 pull RX
	- [x] `wait-rx/poll-rx` 获取 `rx-batch(handle, ...)`
	- [x] `read-desc/read-payload` 按需读取 slice，并按 `shared_mem` 协议 decode
	- [x] decode 后发布 `packet.rx` 事件（component eventbus）
	- [x] `finally { release(handle) }`：成功/失败都必须释放

- [ ] 删除 notify-rx 导出
	- [ ] 移除 `packet-ingest.notify-rx` 的导出实现与所有调用点

- [ ] 验收
	- [ ] 无包时不 busy-loop（使用 `wait-rx` 或合理 timeout/poll 节律）
	- [ ] 单批次异常不会泄漏 handle（lease 过期指标不应持续增长）
	- [ ] decode 失败 / 越界 read / `invalid handle` 等异常路径不允许 panic：必须 `release(handle)` 并继续 loop
	- [ ] 异常路径覆盖：decode 失败/越界 read/invalid handle 时禁止 panic，必须释放并继续 loop

### C. Host（rx-ring provider + NIC RX 入队）

- [x] 实现 `rx-ring` provider（host side）
	- [x] bounded 队列（batch 元数据队列）
	- [x] backing store pool（slot + generation；建议 `handle = (slot_id, generation)`）
	- [x] inflight 管理（handle -> slot + deadline）
	- [x] `poll-rx / wait-rx / read-* / release`
	- [x] `lease timeout` 默认 5000ms + 指标：`rx_ring.lease_expired_total`
	- [x] wait 相关指标（最小集）：`rx_ring.wait_wake_total / rx_ring.wait_timeout_total / rx_ring.wait_shutdown_wake_total`
	- [x] wait 相关指标（建议最小集）：`rx_ring.wait_wake_total` / `rx_ring.wait_timeout_total` / `rx_ring.wait_shutdown_wake_total`

- [x] 队列满策略（写死，默认）：drop newest
	- [x] 队列满丢当前 batch/包，并打点：`rx_ring.enqueue_drop_total`
	- [x] 指标补齐：`rx_ring.queue_depth / rx_ring.inflight_handles / rx_ring.bytes_in_queue`

- [x] 唤醒语义（对应 `wait-rx`）
	- [x] enqueue 新 batch 必须唤醒 `wait-rx`
	- [x] host shutdown 必须唤醒 `wait-rx`（尽快返回 `none`）

- [ ] 验收
	- [ ] lease 超时后旧 handle 稳定 `invalid handle`
	- [ ] backing store 复用不会导致旧 handle 误读新数据（generation 生效）
	- [ ] drop 策略可观测且可定位（可选：按 `sock_id` 分桶统计/采样）
	- [ ] drop 策略可观测：`enqueue_drop_total` 在压测下可解释、可定位（可选：按 sock_id 分桶）

### D. Host（删除 notify-rx 链路）

- [ ] 删除 notify-rx 全链路
	- [x] 删除/废弃 `EngineManager::notify_rx`（及其调用点）（已完成：EngineManager 已降级为 init-only，不再提供运行期注入 API）
	- [ ] 删除/废弃 `ComponentEngine::notify_rx`（及其调用点）
	- [x] 删除 NIC RX 路径中任何“导出注入 RX”调用

- [x] NIC RX 改为只入队
	- [x] 收包 → 编码 desc/payload → `rx_ring.enqueue_batch(...)`

- [ ] 验收
	- [ ] 运行期 host 只有 `scheduler-component.run()` 这一条导出调用
	- [ ] `notify_rx/notify-rx` 不再出现在运行期调用路径（允许文档“现状提示”保留）
	- [ ] guardrail：host 侧任何锁（`std::sync` / `parking_lot` / `tokio`）都不允许跨 wasm call 持有（防止未来引入新的锁序/回调/导出注入变体）
	- [ ] host 侧任何锁都不允许跨 wasm call 持有（run 以外也不允许引入新的导出注入）

## Host Scheduler 最终设计：无饥饿的常驻 RX 轮询

### 核心问题与修复

**问题**：Host scheduler 的 idle wait 机制（用于避免忙等）在某些实现下会导致 NicRx 任务长期得不到轮询机会，表现为收包处理停滞。

**根因**：Bounded idle wait 的内部循环实现会持续 `wait_timeout`，即使超时也不释放锁，导致 `poll_one_resident_task()` 无法获得执行机会。

**修复方案（三层保障）**：

1. **A. Resident-first 策略**：每个 scheduler loop iteration 的第一步就调用 `poll_one_resident_task()`，在任何 idle wait 之前。

2. **B. Bounded idle wait（单次）**：`ingest_blocking_bounded()` 执行一次 `wait_timeout(max_wait=2ms)` 就立即返回（不内部循环），确保锁被及时释放。

3. **C. NicRx 总是合格的**：`poll_one_resident_task()` 中，NicRx 的 backoff 在每次评估前被清空、在每次执行后也被保持清空，确保它永不被抑制。

**效果**：NicRx 保证在每个 2ms 周期内至少被轮询一次，且立即返回（快速 poll 没有锁竞争），从而避免了即使有包也被错过的现象。

### 实现关键点（src/scheduler.rs）

```rust
// 在 Scheduler::run() 主循环中：
self.poll_one_resident_task();  // A: 先轮询 resident（包括 NicRx）

if queues.is_empty() {
    // ... idle 逻辑 ...
    self.ingest_blocking_bounded(&mut queues, IDLE_WAIT_MAX);  // B: 单次等待
    self.poll_one_resident_task();  // 等待后再轮询一次
}

// 在 poll_one_resident_task() 内：
// C: 清空 NicRx 的 backoff，使其总是被选中
for task in state.resident.tasks.iter_mut() {
    if matches!(task.kind, TaskKind::NetworkIo(NetworkIoTask::NicRx)) {
        task.backoff.until = None;
    }
}

// 执行后，如果是 NicRx，也保持 backoff 清空
if is_nicrx {
    entry.backoff.until = None;
    return;  // 不更新 backoff
}
```

### 验收条件

- [x] `no_idle_wait=false`（默认）与 `no_idle_wait=true`（诊断模式）表现等价：NicRx 持续被轮询，包可及时被处理
- [x] 无额外 CPU 占用：Bounded wait 2ms + 其他 resident 使用指数退避保持低频
- [x] 代码规整化 + 文档齐全

### 后续优化空间

如果将来有多个重要的 resident task（不仅 NicRx），可考虑：
- 为不同 resident 类别设置不同的轮询频率保证（例如"网络关键，定时轮询；其他可以退避"）
- 增加 scheduler 侧指标/trace 以观测"实际轮询间隔"是否满足要求

---

### E. 测试与验收（最低配置）

- [ ] 接口一致性
	- [ ] WIT/WAC/composed 导出集合与本文档一致
	- [ ] WIT 变更后代码生成/绑定更新能够编译通过（避免 host/guest binding 不一致）
	- [ ] `wit` 代码生成/绑定更新后编译通过（避免 WIT 变更导致 host/guest binding 不一致）

- [ ] 功能正确性
	- [ ] echo 场景跑通：收包 → `packet.rx` 事件驱动状态机

- [ ] 稳定性
	- [ ] 压测下不出现卡死/死锁；允许 drop，但必须可观测
	- [ ] lease timeout 能兜底回收，不会无限积压
	- [ ] shutdown 验收：host 触发关机后 `wait-rx` 能被唤醒；guest run-loop 能可控停止（或进入可退出态）
	- [ ] shutdown：host 触发关机后，`wait-rx` 能被唤醒；guest run-loop 能在可控时间内停止（或进入 idle 可退出态）

