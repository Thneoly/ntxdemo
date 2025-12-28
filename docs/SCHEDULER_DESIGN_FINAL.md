# Host Scheduler 最终设计总结

## 问题陈述

当 `scheduler.no_idle_wait=false`（生产模式）时，host 收包任务（NicRx）停止被轮询，导致即使有网络包到达，也无法被及时处理。而当 `no_idle_wait=true`（诊断模式）时，收包正常。

**根本原因**：
- `ingest_blocking_bounded()` 的旧实现包含内部 loop，导致即使 `wait_timeout()` 超时也继续阻塞，持续占有 mutex lock。
- 由于 lock 未释放，`poll_one_resident_task()` 无法获得执行机会轮询 NicRx。

## 解决方案：三层保障

### A. Resident-First 策略

在每个 scheduler loop iteration 中，**最先调用 `poll_one_resident_task()`**，在任何 idle wait 之前。

```rust
loop {
    self.poll_one_resident_task();  // ← 最优先
    
    if queues.is_empty() {
        // idle wait 逻辑
    }
}
```

**作用**：确保每次 loop 都有机会轮询 resident task（包括 NicRx）。

---

### B. Bounded Idle Wait（单次，无内部循环）

`ingest_blocking_bounded()` 执行**一次** `wait_timeout(max_wait=2ms)` 就立即返回，**不再内部循环重复等待**。

```rust
fn ingest_blocking_bounded(&self, queues: &mut PriorityQueues, max_wait: Duration) {
    // 1. 非阻塞 drain
    while let Some(task) = state.ingress.pop_front() {
        queues.push(task);
    }
    self.promote_due_timers_locked(&mut state, queues);
    
    if !queues.is_empty() {
        return;  // 有工作，立即返回
    }
    
    // 2. 一次性等待
    let timeout = /* 计算合适超时 */;
    if !timeout.is_zero() {
        let (guard, _) = cv.wait_timeout(state, timeout)?;  // ← 单次
        // 等待返回后，drain 新的任务
        while let Some(task) = guard.ingress.pop_front() { ... }
    }
    
    // 3. 立即返回（不再循环）
}
```

**作用**：
- 确保 lock 被及时释放。
- `poll_one_resident_task()` 能在有限时间内获得执行机会。
- 周期性调用者（main loop）在 wait 后立即调用 `poll_one_resident_task()` 再轮询一次。

---

### C. NicRx 总是合格的

在 `poll_one_resident_task()` 中，NicRx 任务的 backoff 被特殊对待：

1. **进入函数时**：清空 NicRx 的 `backoff.until`，使其总是被选中。
2. **执行后**：即使返回无工作，也保持 `backoff.until = None`，不应用指数退避。

```rust
fn poll_one_resident_task(&self) {
    // 评估前：清空 NicRx 的 backoff
    for task in state.resident.tasks.iter_mut() {
        if matches!(task.kind, TaskKind::NetworkIo(NetworkIoTask::NicRx)) {
            task.backoff.until = None;  // ← 总是合格
        }
    }
    
    // 选择最高优先级的 resident
    let chosen = /* ... */;
    let did_work = self.execute_resident(kind);
    
    // 更新 backoff
    let is_nicrx = matches!(entry.kind, TaskKind::NetworkIo(NetworkIoTask::NicRx));
    if is_nicrx {
        // NicRx 特殊处理：不应用 backoff
        entry.backoff.until = None;
        entry.backoff.current = RESIDENT_BACKOFF_MIN;
        return;
    }
    
    // 其他 resident：应用指数退避
    if did_work {
        entry.backoff.until = None;
        entry.backoff.current = RESIDENT_BACKOFF_MIN;
    } else {
        let next = (entry.backoff.current * 2).min(RESIDENT_BACKOFF_MAX);
        entry.backoff.current = next;
        entry.backoff.until = Some(now + next);  // ← 退避，但 NicRx 不会走这里
    }
}
```

**作用**：
- 确保 NicRx 在每个轮询机会到来时都被执行。
- 其他 resident task 仍然可以使用指数退避来降低 CPU 占用。

---

## 综合效果

Main scheduler loop 伪代码：

```rust
loop {
    // 1. 轮询一次（NicRx 总是被轮询）
    self.poll_one_resident_task();
    
    // 2. 如果队列空，进入 idle wait
    if queues.is_empty() {
        if no_idle_wait {
            // 诊断模式：忙等（1ms sleep）
            self.ingest_nowait(&mut queues);
            std::thread::sleep(Duration::from_millis(1));
        } else {
            // 正常模式：Bounded wait
            if idle_spins < IDLE_SPIN_LIMIT {
                thread::yield_now();
                self.ingest_nowait(&mut queues);
            } else {
                self.ingest_blocking_bounded(&mut queues, 2ms);  // ← 单次等待，立即返回
                self.poll_one_resident_task();  // ← 等待后再轮询一次
            }
        }
    }
    
    // 3. 执行 run queue 中的一个任务
    if let Some(task) = queues.pop() {
        self.execute(task);
    }
}
```

**保证**：
- NicRx 轮询间隔 ≤ 2ms（bounded wait 周期）。
- 不需要忙等，CPU 占用仍然低。
- 同时支持生产模式（`no_idle_wait=false`）和诊断模式（`no_idle_wait=true`）。

---

## 配置说明

`config/app.yaml` 中的 `scheduler.no_idle_wait` 字段：

```yaml
scheduler:
  # 默认值：false（生产模式，使用 bounded idle wait）
  # 可改为：true（诊断模式，忙等，CPU 占用高但轮询最频繁）
  no_idle_wait: false
```

**用途**：
- `false`（默认）：正常生产模式，推荐使用。
- `true`：诊断模式，当怀疑 idle wait 仍有问题时使用。

---

## 代码修改清单

### 文件：`src/scheduler.rs`

1. **顶部模块文档**（新增）：
   - 详细解释设计目标、问题根因、修复方案（A/B/C 三层）。
   - 链接到本文档。

2. **`poll_one_resident_task()` 函数**（改进）：
   - 进入时清空 NicRx 的 backoff（确保总是被选中）。
   - 执行后保持 NicRx 的 backoff 清空（不应用指数退避）。
   - 添加详细注释说明 NicRx 特殊处理的原因。

3. **`ingest_blocking_bounded()` 函数**（修复）：
   - 删除内部 loop，改为单次 `wait_timeout` 后立即返回。
   - 添加详细注释说明"为什么不再内部循环"以及"调用者会再轮询一次"的承诺。

4. **`Scheduler::run()` 主循环**（改进）：
   - 在 `ingest_blocking_bounded()` 返回后添加 `self.poll_one_resident_task()`。
   - 确保 idle wait 后立即再轮询一次。

5. **`ResidentTask` 结构**（整理）：
   - 为 `id` 字段标注 `#[allow(dead_code)]`，文档记录其用途（debug tracing）。

### 文件：`component/doc/HOST.md`

1. **新增小节**："Host Scheduler 最终设计：无饥饿的常驻 RX 轮询"
   - 问题陈述、修复方案、实现关键点、验收条件。
   - 链接回 `docs/SCHEDULER_DESIGN_FINAL.md`。

### 文件：`config/app.yaml`

1. **更新 `scheduler.no_idle_wait` 注释**（已完成）：
   - 明确说明这是诊断开关，不是常态选项。
   - 记录默认值为 `false`（生产模式）。

---

## 验证步骤

1. **编译和单元测试**：
   ```bash
   cargo test -q
   ```
   所有测试应通过，包括 `idle_blocks_until_submit` 等 scheduler 单元测试。

2. **端到端验证**：
   ```bash
   # 确保 config/app.yaml 中 no_idle_wait: false（生产模式）
   cargo run
   # 观察日志，验证 NicRx 持续被轮询
   # 发送测试包，验证能被及时处理
   ```

3. **对标诊断模式**：
   ```bash
   # 临时改为 no_idle_wait: true（诊断模式）
   # 重新运行，验证行为相同（包能被处理）
   # 再改回 false，确认生产模式也正常
   ```

---

## 后续优化方向

1. **多 resident 支持**：
   - 目前只 NicRx 被特殊处理。
   - 若将来有多个关键 resident，可建立一个"轮询优先级"表或动态调整。

2. **观测指标**：
   - 记录"NicRx 实际轮询间隔"，对标 2ms bounded wait。
   - 记录"idle wait 被触发的频率"，观测系统负载。

3. **Self-tuning**：
   - 若观测到轮询间隔超过阈值，自动降低 bounded wait 上限。
   - 若观测到 CPU 占用过高，自动增加其他 resident 的退避。

---

## 总结

通过**三层保障**（resident-first + bounded-single-wait + NicRx-always-eligible），确保了：

✅ NicRx 在生产模式下也能及时轮询（≤2ms 间隔）  
✅ CPU 占用保持低（不忙等，其他 task 使用指数退避）  
✅ 代码清晰、易于维护和未来扩展  
✅ 诊断模式仍可用，便于问题排查  

