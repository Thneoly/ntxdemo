# Sparse Workbook Data Model

## Key Concepts
- **Key Schema**: `user_id/task_id/section/field`
  - Stored as `Vec<u8>` with `0x1F` separator; enables prefix scans per section.
- **Value Payload**: `enum FieldValue { Str(String), I64(i64), F64(f64), Bool(bool) }` serialized via `bincode`.
- **Dirty Flag**: `AtomicU8` per entry; Runner Core writes `1` when field changes and resets after flush.

## Structures
```rust
struct FieldKey {
    user_id: Arc<str>,
    task_id: Arc<str>,
    section: Section,
    field: &'static str,
    version: u32,
}
```
- `version` increments when schema changes; allows backward-compatible decoding.

### HashMap Layout
- Primary store: `HashMap<FieldKey, FieldValue>`
- Secondary index for high-frequency fields (`timeline.phase`, `execution.progress`, `governance.status`).
- For low-frequency sections, entries kept in `Vec<FieldKey>` snapshots every 5 minutes.

## Serialization
- On flush: iterate dirty keys → serialize `(key_bytes, value_bytes, version)` via `bincode`.
- Storage sinks: in-memory ring buffer (progress bus) + optional disk snapshot (`.wbsnap`).

## Cow Fallback
- If timeline Section PoC 未达到 15% 目标，回退策略：
  - 使用 `Cow<'static, str>` 包装字段值，避免复制。
  - 禁用低频快照，仅保留高频 HashMap。

## Open Questions for Design Review
1. 是否需要对 `FieldValue::Str` 应用压缩（如 `lz4`）。
2. 低频快照写入间隔是否可配置（默认 5min）。
3. 是否将 `version` 拆分为 `schema_version` + `field_version`。
