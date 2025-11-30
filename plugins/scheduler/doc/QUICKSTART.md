# Quick Start - Unified Scheduler Component

这是使用 WAC 组合的统一调度器组件的快速开始指南。

## 🚀 快速开始

### 1. 构建统一组件

```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler
./scripts/create_unified.sh
```

**输出**: `composed/target/unified_scheduler.wasm` (430KB)

### 2. 测试组件

```bash
./scripts/test_unified.sh
```

### 3. 查看完整组合方案

```bash
./scripts/compose_full.sh
```

## 📦 当前功能

统一组件目前包含 **scheduler-core** (core-libs):

### 类型定义

```wit
record scenario {
    version: string,
    name: string,
    description: option<string>,
    // ...
}

record action-def {
    id: string,
    action-type: string,
    // ...
}
```

### 解析函数

```wit
parse-scenario: func(yaml: string) -> result<scenario, string>
validate-scenario: func(scenario: scenario) -> result<_, string>
```

## 🔍 检查组件

### 查看接口

```bash
wasm-tools component wit composed/target/unified_scheduler.wasm
```

### 验证组件

```bash
wasm-tools validate composed/target/unified_scheduler.wasm
```

### 查看导出

```bash
wasm-tools component wit composed/target/unified_scheduler.wasm | grep export
```

**输出**:
```
export scheduler:core-libs/types@0.1.0
export scheduler:core-libs/parser@0.1.0
```

## 📊 组件信息

| 属性 | 值 |
|------|-----|
| 文件名 | unified_scheduler.wasm |
| 大小 | 430KB |
| 架构 | wasm32-wasip2 |
| 状态 | ✅ 可用 |

## 🔧 脚本说明

### `create_unified.sh`

主要组合脚本：
1. 构建 core-libs 组件
2. 创建统一组件
3. 验证输出

### `test_unified.sh`

测试脚本：
1. 检查组件是否存在
2. 显示组件信息
3. 验证组件结构
4. 列出导出接口

### `compose_full.sh`

演示脚本：
1. 显示当前状态
2. 展示完整组合计划
3. 说明未来步骤

## 🎯 使用示例

### 示例 1: 解析 YAML 场景

```yaml
# example_scenario.yaml
version: "1.0"
name: "test-workflow"
description: "Simple test"
workflows:
  nodes:
    - id: "step1"
      type: "action"
      name: "First Step"
```

### 示例 2: 与 Wasmtime 集成

```rust
use wasmtime::component::*;
use wasmtime::{Engine, Store, Config};

let mut config = Config::new();
config.wasm_component_model(true);
let engine = Engine::new(&config)?;

let component = Component::from_file(
    &engine, 
    "composed/target/unified_scheduler.wasm"
)?;

// ... 实例化并调用
```

## 📁 目录结构

```
plugins/scheduler/
├── composed/
│   ├── socket.wac              # WAC 组合配置
│   ├── world.wit               # 统一接口定义
│   └── target/
│       └── unified_scheduler.wasm  # 统一组件 (430KB)
│
├── core-libs/                  # ✅ 已完成
│   ├── wit/
│   │   └── world.wit
│   └── target/wasm32-wasip2/release/
│       └── scheduler_core.wasm
│
├── executor/                   # 🚧 进行中
│   └── wit/
│       └── world.wit
│
├── actions-http/               # 🚧 待完成
│   └── wit/
│       └── world.wit
│
├── create_unified.sh           # 主构建脚本
├── test_unified.sh             # 测试脚本
├── compose_full.sh             # 演示脚本
│
├── WAC_COMPOSITION.md          # 详细文档
├── USAGE.md                    # 使用指南
└── QUICKSTART.md               # 本文件
```

## 🛠️ 工具要求

已安装工具：
- ✅ `cargo-component` - 构建 wasm 组件
- ✅ `wasm-tools` - 验证和检查组件
- ✅ `wac` - 组合多个组件

验证安装：
```bash
cargo component --version
wasm-tools --version
wac --version
```

## 🎯 当前状态

| 组件 | 状态 | 说明 |
|------|------|------|
| core-libs | ✅ 完成 | 类型定义和解析器 |
| executor | 🚧 进行中 | 需要 Guest trait 实现 |
| actions-http | 🚧 待完成 | 等待 executor |
| unified | ✅ 可用 | 当前包含 core-libs |

## 📈 下一步

### 优先级 1: 修复 Executor

```bash
cd core-libs
cargo component build --target wasm32-wasip2
# 修复 Guest trait 实现
```

### 优先级 2: 完成 Actions-HTTP

```bash
cd actions-http
cargo component build --target wasm32-wasip2
```

### 优先级 3: 完整组合

```bash
./scripts/compose_full.sh  # 执行实际的 wac plug 命令
```

## 🆘 故障排除

### 组件不存在

```bash
./scripts/create_unified.sh  # 重新构建
```

### 验证失败

```bash
wasm-tools validate composed/target/unified_scheduler.wasm
```

### 查看详细接口

```bash
wasm-tools component wit composed/target/unified_scheduler.wasm | less
```

## 📖 更多文档

- **WAC_COMPOSITION.md**: WAC 组合详细说明
- **USAGE.md**: 集成指南和 API 示例
- **COMPONENTS.md**: 组件架构设计
- **README.md**: 项目总览

## ✅ 验证清单

- [x] 组件构建成功
- [x] 组件验证通过
- [x] 接口正确导出
- [x] 测试脚本工作
- [x] 文档完整
- [ ] Executor 实现完成
- [ ] Actions-HTTP 实现完成
- [ ] 完整 WAC 组合完成

## 🎉 完成

你现在有了一个工作的统一调度器组件！虽然目前只包含 core-libs 功能，但基础设施已经完备，可以轻松添加更多组件。

运行 `./scripts/test_unified.sh` 查看当前状态！
