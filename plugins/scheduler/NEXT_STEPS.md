# WASM 组件组合 - 下一步行动方案

## 当前状态

### 已完成
1. ✅ scheduler.wasm 已构建 (16MB)
2. ✅ scheduler_actions_http.wasm 已构建 (14MB)
3. ✅ scheduler_core.wasm 已构建 (11MB)
4. ✅ scheduler_executor.wasm 已构建 (11MB)

### 核心问题
scheduler.wasm 导入了自定义接口：
```
import scheduler:core-libs/wasi-network@0.1.0
import scheduler:core-libs/wasi-tcp@0.1.0
import scheduler:core-libs/wasi-udp@0.1.0
```

这些接口需要由某个组件提供实现，但：
- wasmtime 只提供标准 WASI 接口 (wasi:sockets/*)
- 没有组件实现这些自定义接口
- wasm-tools compose 无法找到依赖

## 解决方案选项

### 方案 A: 简化 - 移除 WIT 组件模式（推荐，最快）

**策略**：scheduler、executor、actions-http 通过 Rust 依赖静态链接 core-libs，不使用 WIT 接口分离。

**步骤**：
1. 移除 core-libs/src/component.rs
2. 移除 executor 的 WIT 接口定义
3. 移除 actions-http 的 WIT 接口定义
4. scheduler 作为单一 WASM 组件，内部所有依赖都是 Rust 级别的静态链接
5. scheduler.wasm 只导入标准 WASI 接口

**优点**：
- 简单直接
- 不需要 WAC 组合
- 构建速度快
- 已经部分完成

**缺点**：
- 失去组件化的灵活性
- 无法单独替换 action 组件

### 方案 B: 完整 WASI 适配（复杂）

**策略**：创建 WASI 适配器组件，桥接标准 WASI sockets 到自定义接口。

**步骤**：
1. 创建 wasi-socket-adapter 组件
2. 适配器导入 `wasi:sockets/*`
3. 适配器导出 `scheduler:core-libs/wasi-*`
4. 使用 WAC 组合：adapter + scheduler

**优点**：
- 保持组件化架构
- 可以单独替换组件

**缺点**：
- 需要编写适配器代码
- 复杂度高
- 调试困难

### 方案 C: 直接使用标准 WASI（中等复杂度）

**策略**：修改 core-libs 的 socket 实现，在 WASM 下直接调用标准 WASI sockets API。

**步骤**：
1. 修改 core-libs/src/socket/mod.rs
2. 在 `#[cfg(target_arch = "wasm32")]` 下使用 wasi::sockets
3. 移除自定义的 wasi-network/wasi-tcp 接口定义
4. 重新构建所有组件

**优点**：
- 使用标准接口
- wasmtime 可以直接提供实现
- 不需要适配器

**缺点**：
- 需要修改 socket 实现代码
- 需要处理 WASI sockets 的异步模型

## 推荐方案：方案 A（简化）

### 具体实施步骤

#### 1. 清理 core-libs
```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/core-libs

# 移除 component.rs
rm src/component.rs

# 修改 lib.rs，移除 component 模块
# 移除 WIT world 定义中的 export
```

#### 2. 修改 core-libs/Cargo.toml
```toml
[lib]
crate-type = ["rlib"]  # 只编译为 Rust 库，不是 cdylib

# 移除 wit-bindgen 依赖
# 移除 [package.metadata.component]
```

#### 3. 简化 scheduler WIT
scheduler/wit/world.wit:
```wit
package scheduler:main;

world scheduler-component {
    // 只导入标准 WASI 接口（wasmtime 可以提供）
    import wasi:cli/environment@0.2.6;
    import wasi:cli/exit@0.2.6;
    import wasi:cli/stdout@0.2.6;
    import wasi:cli/stderr@0.2.6;
    import wasi:clocks/monotonic-clock@0.2.6;
    
    // 只导出 run-scenario 函数
    export run-scenario: func(scenario-yaml: string) -> result<string, string>;
}
```

#### 4. 修改 scheduler 的 socket 使用
在 scheduler 内部，通过 Rust 代码调用 core-libs 的 socket API，
core-libs 在 WASM 环境下使用 stub 实现或 WASI bindings。

#### 5. 重新构建
```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/scheduler
cargo component build --lib --target wasm32-wasip2
```

#### 6. 测试
```bash
wasmtime run scheduler.wasm \
    --invoke 'run-scenario' \
    -- "$(cat test_scenario.yaml)"
```

### 预期结果
- scheduler.wasm 大小：~5-10MB（包含所有依赖）
- 只导入标准 WASI 接口
- wasmtime 可以直接运行
- 无需 WAC 组合

## 当前状态文件

已创建的组件（但有依赖问题）：
- `/home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/target/wasm32-wasip2/debug/scheduler.wasm`
- `/home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/target/wasm32-wasip2/debug/scheduler_core.wasm`
- `/home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/target/wasm32-wasip2/debug/scheduler_executor.wasm`
- `/home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/target/wasm32-wasip2/debug/scheduler_actions_http.wasm`

## 下一步行动

**立即执行：方案 A - 简化架构**

1. 移除 core-libs 的组件模式
2. 简化 scheduler WIT 定义
3. 重新构建并测试

**时间估计：30分钟**
