# wit-bindgen 0.48 兼容性修复

## 概述
升级到 wit-bindgen 0.48 后，scheduler 插件的构建出现了编译错误。本文档记录了遇到的问题和解决方案。

## 遇到的问题

### 1. Executor 编译错误
**错误信息:**
```
error[E0277]: the trait bound `SchedulerExecutorImpl: context::Guest` is not satisfied
```

**原因:** 
在 wit-bindgen 0.48 中，如果 WIT 定义导出了 `resource` 类型（如 `action-context`），则必须实现相应的 `Guest` trait 并提供资源的实现类型。

**解决方案:**
1. 实现 `exports::scheduler::executor::context::Guest` trait
2. 创建 `ActionContextImpl` 结构体实现 `GuestActionContext` trait
3. 实现所有资源方法（constructor, register-action, add-task 等）

修改的文件: `executor/src/component.rs`

### 2. Actions-HTTP 依赖问题
**错误信息:**
```
error occurred in cc-rs: failed to find tool "clang": No such file or directory
```

**原因:**
`ureq` HTTP 客户端依赖 `ring` crate，而 `ring` 需要 clang 编译器进行原生代码编译。在 WASM 目标中不应使用这类依赖。

**解决方案:**
1. 将 `ureq` 依赖移到条件编译下，仅在非 WASM 目标时启用
2. 使用条件编译将 `HttpActionComponent` 限制为非 WASM 目标
3. WASM 组件实现使用 stub（将来应使用 wasi:http）

修改的文件:
- `actions-http/Cargo.toml` - 添加条件依赖
- `actions-http/src/lib.rs` - 添加条件编译
- `actions-http/src/component.rs` - 修复导入路径和类型引用

## 技术细节

### wit-bindgen 0.48 的变化

#### Resource 处理
在 0.48 版本中，导出的 resource 需要完整的实现：

```rust
// 实现 Guest trait 提供 Resource 类型
impl exports::scheduler::executor::context::Guest for SchedulerExecutorImpl {
    type ActionContext = ActionContextImpl;
}

// 实现 Resource 的具体类型
struct ActionContextImpl {
    // 内部状态
}

// 实现 Resource 的所有方法
impl exports::scheduler::executor::context::GuestActionContext for ActionContextImpl {
    fn new() -> Self { ... }
    fn register_action(&self, action: ActionDef) { ... }
    fn add_task(&self, task: WbsTask) { ... }
    // ... 其他方法
}
```

#### 导出命名空间
wit-bindgen 0.48 使用更详细的命名空间路径：
- 旧版本: `exports::types::ActionDef`
- 0.48: `exports::scheduler::executor::types::ActionDef`

### 条件编译模式

为了支持同时构建 WASM 组件和本地库，使用条件编译：

```toml
# Cargo.toml
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
ureq = { version = "3.1.3", features = ["json"] }
```

```rust
// lib.rs
#[cfg(not(target_arch = "wasm32"))]
use ureq::Agent;

#[cfg(not(target_arch = "wasm32"))]
pub struct HttpActionComponent {
    agent: Agent,
}
```

## 构建结果

修复后，所有三个组件成功构建：

```
✓ scheduler_core.wasm (431KB)
  - export scheduler:core-libs/types@0.1.0
  - export scheduler:core-libs/parser@0.1.0

✓ scheduler_executor.wasm (444KB)
  - export scheduler:executor/types@0.1.0
  - export scheduler:executor/context@0.1.0
  - export scheduler:executor/component-api@0.1.0
  - 包含 core-libs 的导出

✓ scheduler_actions_http.wasm (594KB)
  - export scheduler:actions-http/types@0.1.0
  - export scheduler:actions-http/http-component@0.1.0
  - 包含 executor 和 core-libs 的导出
```

## 验证

所有组件通过 `wasm-tools validate` 验证：
```bash
wasm-tools validate target/wasm32-wasip2/release/scheduler_core.wasm
wasm-tools validate target/wasm32-wasip2/release/scheduler_executor.wasm
wasm-tools validate target/wasm32-wasip2/release/scheduler_actions_http.wasm
```

## 后续工作

1. **实现真实的 HTTP 功能**: 当前 actions-http 使用 stub 实现，应该使用 wasi:http 接口实现真实的 HTTP 请求
2. **完善 ActionContext**: executor 的 ActionContext 当前是空实现，需要添加实际的事件队列和状态管理
3. **组合测试**: 测试三个组件的组合和互操作性

## 参考资料

- wit-bindgen 0.48 发布说明
- WebAssembly Component Model 规范
- WASI Preview 2 文档

---
修复日期: 2025-11-30
修复版本: wit-bindgen 0.48.1
