# Core Libraries 模块结构

本目录包含 scheduler 的核心库模块，每个模块位于独立的文件夹中。

## 模块说明

### dsl/
DSL (Domain Specific Language) 解析器模块
- 解析 YAML 格式的工作流定义
- 场景（Scenario）数据结构
- 提供 `parse_scenario()` 和 `validate_scenario()` 接口

### error/
错误类型定义
- `SchedulerError` - 统一的错误类型
- 错误处理工具

### ip/
IP 地址池管理模块
- IP 范围定义（支持 CIDR 表示法）
- IP 分配和释放
- 支持绑定 IP 到资源（通过 subinstance/subid/subtype）
- 支持多种资源类型：MAC 地址、VM、容器、Pod 等
- 保留 IP 管理
- 池统计信息

详细使用说明参见：`doc/IP_POOL_USAGE.md`

### socket/
网络 socket 抽象层
- **mod.rs** - 公共 socket API
- **wasi_impl.rs** - WASI Preview 2 socket 实现
- **socket_stub.rs** - Stub 实现（历史）
- **socket_mixed.rs.bak** - 混合实现备份

支持的功能：
- TCP 客户端/服务器
- UDP 数据报
- IPv4/IPv6 地址
- WASI Preview 2 网络接口

### state_machine/
状态机实现
- 状态转换逻辑
- 事件处理

### wbs/
WBS (Work Breakdown Structure) 工作分解结构
- WBS 树数据结构
- 任务节点和边
- 任务类型（WbsTaskKind）

### workbook/
Workbook 工作簿管理
- 工作簿数据结构
- 工作簿操作

## 构建

```bash
cargo component build --release
```

## 导出接口

组件导出以下 WIT 接口：
- `scheduler:core-libs/types@0.1.0` - 核心数据类型
- `scheduler:core-libs/parser@0.1.0` - DSL 解析器
- `scheduler:core-libs/socket@0.1.0` - Socket API

## 导入接口

组件导入 WASI socket 接口：
- `scheduler:core-libs/wasi-network@0.1.0` - 网络资源
- `scheduler:core-libs/wasi-tcp@0.1.0` - TCP socket
- `scheduler:core-libs/wasi-udp@0.1.0` - UDP socket

加上标准 WASI Preview 2 接口（cli, io, filesystem 等）
