# HTTP 动态任务与事件总线指南

本文档说明如何让 `actions-executor` 组件在成功执行后，通过事件总线向调度器注入新的任务（`SchedulerEvent::AddTask`），并给出端到端验证步骤。

## 动态任务工作原理

1. `actions-executor` 在成功执行任意 HTTP 动作后，会检查 `with.dynamic_tasks` 字段。
2. 每个条目会被编码为 `WbsTask` 并经由 `scheduler:event-bus/event-bus` 接口推送到宿主。
3. 调度器在每次动作执行后都会主动 `drain` 事件总线，将收到的任务写入运行中的 WBS，再调度执行。
4. 当前实现默认限制每次最多处理 8 个动态任务，防止恶意或意外的任务风暴。

## YAML 片段示例

```yaml
actions:
  actions:
    - id: seed-dynamic
      call: get
      with:
        url: "http://{{resource.ip}}:{{resource.port}}/{{undefined.endpoint}}"
        dynamic_tasks:
          - id: "dyn-followup-{{user.id}}-{{user.iteration}}"
            action_id: follow-up
            kind: action
            outgoing:
              - target: dyn-end
                label: done
    - id: follow-up
      call: get
      with:
        url: "http://{{resource.ip}}:{{resource.port}}/{{undefined.endpoint}}"
```

- `id`: 新任务节点 ID，建议包含用户/迭代信息以避免冲突。
- `action_id`: 必须引用 `actions.actions` 中已存在的动作。
- `kind`: 目前支持 `action` 和 `end`。
- `outgoing`: 可选，描述新增任务结束后指向的边。

完整示例可参考 `res/http_dynamic_tasks.yaml`。

## 端到端验证

1. 先编译并同步组件：

```bash
./scripts/run_scheduler_component.sh res/http_dynamic_tasks.yaml
```

该脚本会：

- 使用 `cargo build --target wasm32-wasip2` 构建 core-libs / scheduler / actions-executor / eventbus。
- 将最新的 WASM 复制到 `wac/deps/scheduler/`，包含 `actions-executor.wasm`、`core-libs.wasm`、`event-bus.wasm` 和 `main.wasm`。
- 通过 `wac compose wac/scheduler-composition.wac --deps-dir wac/deps` 重新生成组合组件，并用 wasmtime 执行传入的场景文件。

2. 运行结束后可以在输出中看到：

```
Scenario: http_dynamic_tasks
...
Total actions executed: 2
```

第 2 次动作即为通过事件总线注入的 `follow-up` 任务。

## 常见问题排查

- **没有触发新任务**：确认 `actions-executor` 返回状态为 Success，且 `dynamic_tasks` 字段渲染后的 JSON 确实是数组。可以在 YAML 中引用不存在的模板变量（例如 `{{undefined.endpoint}}`）来让示例跳过真实网络调用但仍然返回 Success。
- **WAC 组合报错缺少 event-bus**：确保脚本已将 `eventbus.wasm` 复制为 `wac/deps/scheduler/event-bus.wasm`，并在 `wac/scheduler-composition.wac` 中为 scheduler 与 actions-executor 都导入 `scheduler:event-bus/event-bus@0.1.0`。
- **重复任务 ID**：调度器只会执行一次同名任务；如果需要多次注入，请在 `id` 中带上用户或迭代信息。
