# Directory Structure

本文档说明 scheduler 项目的目录组织结构。

## 📁 顶层目录结构

```
plugins/scheduler/
├── README.md               # 项目主文档
├── Cargo.toml              # Workspace 配置
│
├── doc/                    # 📚 所有文档
├── scripts/                # 🔧 所有脚本
├── wac/                    # 📦 WAC 组合配置
├── examples/               # 💡 示例代码
│
├── core-libs/              # ✅ Core 组件源码
├── executor/               # 🚧 Executor 组件源码
├── actions-http/           # 🚧 HTTP Actions 组件源码
├── scheduler/              # 📦 主调度器二进制
│
├── composed/               # 🎯 组合输出
│   ├── world.wit           # 统一接口定义
│   ├── deps.toml           # 依赖配置
│   └── target/
│       └── unified_scheduler.wasm  # 统一组件 (430KB)
│
└── target/                 # 构建输出（Cargo 标准）
```

## 📚 doc/ - 文档目录

所有用户文档、技术文档、参考文档都存放在这里。

```
doc/
├── INDEX.md                # 📖 文档导航索引
├── QUICKSTART.md           # 🚀 5分钟快速开始
├── SUMMARY.md              # 📊 项目总结和成就
├── ARCHITECTURE.md         # 🏗️ 架构图表
├── COMPONENTS.md           # 🔧 组件设计详情
├── WAC_COMPOSITION.md      # 🔗 WAC 组合技术细节
├── USAGE.md                # 📦 API 使用指南
├── FILE_INDEX.md           # 📁 完整文件索引
├── DIRECTORY_STRUCTURE.md  # 📂 本文档
└── draft.md                # 草稿（遗留）
```

### 文档阅读顺序

**新用户**:
1. README.md (项目根目录)
2. doc/INDEX.md
3. doc/QUICKSTART.md

**开发者**:
1. doc/COMPONENTS.md
2. doc/ARCHITECTURE.md
3. doc/WAC_COMPOSITION.md

**集成工程师**:
1. doc/USAGE.md
2. examples/use_unified.rs

## 🔧 scripts/ - 脚本目录

所有自动化脚本都存放在这里。

```
scripts/
├── create_unified.sh           # 🎯 主构建脚本 - 创建统一组件
├── test_unified.sh             # ✅ 测试验证脚本
├── compose_full.sh             # 📋 完整组合方案演示
├── compose_demo.sh             # 💡 组合示例演示
├── compose.sh                  # 🔧 WAC 组合脚本
└── build_all_components.sh     # 🏗️ 批量构建所有组件
```

### 脚本使用

所有脚本都应该从项目根目录运行：

```bash
cd /path/to/plugins/scheduler

# 构建统一组件
./scripts/create_unified.sh

# 测试组件
./scripts/test_unified.sh

# 查看完整方案
./scripts/compose_full.sh
```

## 📦 wac/ - WAC 配置目录

所有 WAC (WebAssembly Composition) 配置文件。

```
wac/
├── composition.wac         # 完整三组件组合配置
├── unified-simple.wac      # 简化单组件包装配置
└── (future: more .wac files)
```

### WAC 文件说明

| 文件 | 用途 | 状态 |
|------|------|------|
| composition.wac | 完整组合（3个组件） | 🚧 待executor和actions-http完成 |
| unified-simple.wac | 简化包装（仅core-libs） | ✅ 可用 |

## 💡 examples/ - 示例目录

代码示例和集成演示。

```
examples/
├── Cargo.toml              # 示例项目配置
└── use_unified.rs          # Rust 集成示例
```

### 添加新示例

1. 在 examples/ 中创建新文件
2. 在 examples/Cargo.toml 中添加 [[example]] 条目
3. 使用 `cargo run --example <name>` 运行

## 🏗️ 组件源码目录

### core-libs/ - ✅ 核心库组件

```
core-libs/
├── Cargo.toml              # 包配置
├── build.sh                # 构建脚本
├── src/
│   ├── lib.rs              # 主库入口
│   ├── component.rs        # 组件绑定实现
│   ├── dsl.rs              # DSL 解析
│   ├── state_machine.rs    # 状态机
│   ├── wbs.rs              # WBS 树
│   └── ...
└── wit/
    ├── world.wit           # WIT 接口定义
    └── deps.toml           # WIT 依赖
```

**状态**: ✅ 完全可用
**输出**: `target/wasm32-wasip2/release/scheduler_core.wasm` (~300KB)

### executor/ - 🚧 执行器组件

```
executor/
├── Cargo.toml
├── build.sh
├── src/
│   ├── lib.rs
│   ├── component.rs        # 🚧 需要实现 Guest traits
│   └── ...
└── wit/
    ├── world.wit           # WIT 接口定义
    └── deps.toml
```

**状态**: 🚧 WIT 完成，Rust 实现需完善
**待完成**: Guest trait 实现

### actions-http/ - 🚧 HTTP 动作组件

```
actions-http/
├── Cargo.toml
├── build.sh
├── src/
│   ├── lib.rs
│   ├── component.rs        # 🚧 等待 executor
│   └── ...
└── wit/
    ├── world.wit
    └── deps.toml
```

**状态**: 🚧 等待 executor 完成
**依赖**: executor 组件

### scheduler/ - 📦 主二进制

```
scheduler/
├── Cargo.toml
└── src/
    ├── main.rs             # 主调度器 CLI
    ├── engine.rs           # 调度引擎
    ├── lib.rs              # 库入口
    └── bin/
        └── http_server.rs  # HTTP 测试服务器
```

**状态**: ✅ 原生模式可用
**用途**: 非 WASM 模式下的完整调度器

## 🎯 composed/ - 组合输出

统一组件的输出目录。

```
composed/
├── world.wit               # 统一组件接口定义
├── deps.toml               # 依赖配置
└── target/
    └── unified_scheduler.wasm  # ✅ 430KB 统一组件
```

### 组件信息

- **文件**: unified_scheduler.wasm
- **大小**: 430KB (当前), ~800KB (完整版)
- **当前内容**: scheduler-core (core-libs)
- **计划内容**: core-libs + executor + actions-http

## 🏗️ target/ - 构建输出

Cargo 标准构建输出目录。

```
target/
├── debug/                  # Debug 构建
│   ├── scheduler           # 主二进制
│   ├── http_server         # HTTP 服务器
│   └── ...
├── release/                # Release 构建
└── wasm32-wasip2/          # WASM 组件构建
    ├── debug/
    └── release/
        ├── scheduler_core.wasm          # ✅
        ├── scheduler_executor.wasm      # 🚧
        └── scheduler_actions_http.wasm  # 🚧
```

## 📋 文件命名约定

### 文档

- 全大写 + .md: `README.md`, `QUICKSTART.md`, `SUMMARY.md`
- 存放位置: `doc/`

### 脚本

- 小写 + 下划线 + .sh: `create_unified.sh`, `test_unified.sh`
- 存放位置: `scripts/`
- 必须可执行: `chmod +x scripts/*.sh`

### WAC 文件

- 小写 + 连字符 + .wac: `composition.wac`, `unified-simple.wac`
- 存放位置: `wac/`

### 组件输出

- 小写 + 下划线 + .wasm: `scheduler_core.wasm`, `unified_scheduler.wasm`
- 存放位置: 
  - 单个组件: `target/wasm32-wasip2/release/`
  - 统一组件: `composed/target/`

## 🔍 快速查找

### 查找文档

```bash
# 列出所有文档
ls doc/*.md

# 搜索特定主题
grep -r "WAC composition" doc/
```

### 查找脚本

```bash
# 列出所有脚本
ls scripts/*.sh

# 查看脚本功能
head -5 scripts/*.sh
```

### 查找组件

```bash
# 列出所有 WASM 组件
find . -name "*.wasm" -type f

# 查看统一组件
ls -lh composed/target/unified_scheduler.wasm
```

## 🧹 清理命令

### 清理构建产物

```bash
# 清理 Cargo 构建
cargo clean

# 清理特定目标
rm -rf target/wasm32-wasip2/
rm -rf composed/target/
```

### 清理临时文件

```bash
# 清理编辑器临时文件
find . -name "*~" -delete
find . -name "*.swp" -delete

# 清理 git 忽略的文件
git clean -fdx
```

## ✅ 验证文件组织

运行以下命令验证文件组织正确：

```bash
# 检查文档
test -d doc && echo "✅ doc/ exists"
test -f doc/INDEX.md && echo "✅ doc/INDEX.md exists"

# 检查脚本
test -d scripts && echo "✅ scripts/ exists"
test -x scripts/create_unified.sh && echo "✅ scripts/create_unified.sh is executable"

# 检查 WAC 文件
test -d wac && echo "✅ wac/ exists"
test -f wac/composition.wac && echo "✅ wac/composition.wac exists"

# 检查组件
test -f composed/target/unified_scheduler.wasm && echo "✅ unified component exists"
```

## 📊 目录统计

```bash
# 文档数量
ls doc/*.md | wc -l
# 预期: 9个文档

# 脚本数量
ls scripts/*.sh | wc -l
# 预期: 6个脚本

# WAC 文件数量
ls wac/*.wac | wc -l
# 预期: 2个 WAC 文件

# 组件数量
find . -name "*.wasm" -type f | wc -l
# 预期: 至少 1个 (unified_scheduler.wasm)
```

## 🔄 迁移说明

如果你有旧版本的项目结构，进行以下迁移：

```bash
# 1. 移动脚本
mkdir -p scripts
mv *.sh scripts/

# 2. 移动 WAC 文件
mkdir -p wac
mv *.wac wac/

# 3. 移动文档
mv ARCHITECTURE.md COMPONENTS.md FILE_INDEX.md INDEX.md \
   QUICKSTART.md SUMMARY.md USAGE.md WAC_COMPOSITION.md doc/

# 4. 更新文档中的引用
cd doc
sed -i 's|\./create_unified\.sh|./scripts/create_unified.sh|g' *.md
sed -i 's|\./test_unified\.sh|./scripts/test_unified.sh|g' *.md
# ... 其他脚本
```

## 📝 维护指南

### 添加新文档

1. 在 `doc/` 中创建新的 .md 文件
2. 在 `doc/INDEX.md` 中添加链接
3. 在 `README.md` 的文档部分添加引用（如果需要）

### 添加新脚本

1. 在 `scripts/` 中创建新的 .sh 文件
2. 设置可执行权限: `chmod +x scripts/new_script.sh`
3. 在 `doc/FILE_INDEX.md` 中记录
4. 确保脚本从项目根目录运行

### 添加新 WAC 配置

1. 在 `wac/` 中创建新的 .wac 文件
2. 在相关文档中说明其用途
3. 可选：创建对应的构建脚本

---

**版本**: 1.0
**最后更新**: 2024-11-30
**维护者**: Scheduler Team
