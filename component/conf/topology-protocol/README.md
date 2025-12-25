### 拓扑变更事件协议（`topology.changed`）

本协议用于在运行时通过 eventbus 变更 scheduler 的 workflow/workbook/actions 拓扑。

#### 设计约束
- **只影响新 user**：旧 user 绑定 `scenario_version` 不迁移。
- **严格协议**：payload 必须符合 schema（未知字段将被拒绝）。
- **并发控制**：`patch` 必须基于当前 active 版本（`base_version == active_version`），否则拒绝。

---

### 事件：`topology.changed`

`Event.kind = "topology.changed"`，`Event.payload` 为 JSON 字符串，格式如下：

```json
{
  "schema_version": 1,
  "change_id": "chg-001",
  "mode": "replace-yaml",
  "base_version": 1,
  "scenario_yaml": "version: v1\nname: ...\nworkflows: ...\n"
}
```

#### 公共字段
- `schema_version`（必填）：当前固定为 `1`
- `change_id`（必填）：变更 ID（建议全局唯一，便于审计/回放）
- `mode`（必填）：`replace-yaml | replace-json | patch`
- `base_version`（可选）：
  - `patch` 时建议必填，并且必须等于当前 active_version（否则拒绝）
  - replace 模式可填可不填（仅用于审计字段）

---

### mode：replace-yaml

字段：
- `scenario_yaml`（必填，string）：完整 Scenario（YAML 或 JSON 文本均可，scheduler 会尝试解析）

示例：见 `payloads/replace-yaml.json`

---

### mode：replace-json

字段：
- `scenario_json`（必填，object）：完整 Scenario JSON 对象（等价于 scenario.yaml 解析后的结构）

示例：见 `payloads/replace-json.json`

---

### mode：patch

字段：
- `ops`（必填，array）：按顺序应用的 diff 操作数组

示例：见 `payloads/patch.json`

#### 支持的 ops（当前实现）
- `set-node-priority`
- `upsert-edge`
- `remove-node`
- `add-node`
- `upsert-action`

> 注意：patch 应用后会执行完整 `validate_scenario()` 校验，失败则拒绝。

---

### 输出事件

#### `scheduler.topology.applied`
payload 字段：
- `change_id`
- `base_version`
- `new_version`
- `mode`

#### `scheduler.topology.rejected`
payload 字段：
- `change_id`（若可解析到）
- `base_version` / `active_version`（若可解析到）
- `mode`（若可解析到）
- `error`


