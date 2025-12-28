
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

