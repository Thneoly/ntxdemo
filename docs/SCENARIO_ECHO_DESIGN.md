# NIC-HOST-Guest Echo 场景实现方案

## 概述

本文档描述如何实现一个完整的 Echo 场景，使用 WAC 组装模块化的 Wasm 组件。

### 架构层级

```
Host-1（Echo Server）                 Host-2（Echo Client）

 ┌─────────────────────────┐          ┌─────────────────────────┐
 │ NIC Layer + Decoder     │          │ NIC Layer + Generator   │
 └────────┬────────────────┘          └────────┬────────────────┘
          │                                    │
          ▼                                    │
 ┌─────────────────────────┐                  │
 │ Echo Server Wasm        │◄──────────────────┘
 │ (WAC 组装)              │
 │ ├─ corelibs             │  Socket、日志等基础
 │ ├─ eventbus             │  消息总线
 │ ├─ actions-executor     │  ★ 包含 UDP echo 处理逻辑
 │ └─ scheduler            │  调度协调
 └─────────────────────────┘
          │
          ▼
 ┌─────────────────────────┐
 │ Reply Builder           │
 └────────┬────────────────┘
          │
          ▼
       网络
```

### 关键设计原理

| 组件 | 职责 | 说明 |
|------|------|------|
| **corelibs** | 基础接口库 | Socket 操作、日志、数据结构 |
| **eventbus** | 消息总线 | Host ↔ Scheduler 和 Scheduler ↔ Actions 的事件传递 |
| **actions-executor** | 动作执行器 | ★ **包含 UDP echo 的业务逻辑** |
| **scheduler** | 调度器 | 协调组件间的消息流和任务调度 |
| **WAC** | 组件编排 | 将上述组件组装成完整的 Echo Server/Client Wasm |

### 端到端流程

```
数据流向：
  Host 网络包 
    ↓
  NIC Layer.recv() 
    ↓ 
  Packet Decoder 
    ↓ 
  PacketMeta + Payload 
    ↓ 
  [Wasm组件（由WAC组装）]
    ├─ eventbus: emit(PacketReceived)
    ├─ scheduler: dispatch_task()
    ├─ actions-executor: handle_echo() ★ 业务逻辑
    ├─ eventbus: emit(ReplyReady)
    └─ 返回 response_payload
    ↓ 
  Reply Builder 
    ↓ 
  NIC Layer.send()
```

---

## 1. 架构设计

### 1.1 Phase 1：WAC 组装双组件架构

#### 方案说明

**核心原则**：Echo Server 和 Echo Client **使用相同的 4 个组件库**（corelibs、eventbus、actions-executor、scheduler），但通过 **不同的 WAC 编排配置** 生成两个不同的 .wasm 文件。关键差异在于 **actions-executor 的业务逻辑实现完全不同**。

**Echo Server Wasm 组装**：

```
📦 echo-server.wasm (由 WAC 编排生成)
│
├─ corelibs.wasm
│  ├─ socket API（监听、发送）
│  ├─ log()
│  └─ 数据结构定义
│
├─ eventbus.wasm
│  ├─ emit_packet_received(meta, payload)
│  ├─ emit_task_complete(response)
│  └─ on_event_handler()
│
├─ 🎯 actions-executor-server.wasm
│  ├─ handle_on_packet_received(meta, payload)
│  │  └─ ⭐ 业务逻辑 1：接收 PacketReceived 事件
│  │      ├─ 解析入站包的 payload
│  │      ├─ ✅ echo = payload（内容不变）
│  │      ├─ 调用 eventbus.emit(TaskComplete)
│  │      └─ 返回 response_payload
│  │
│  └─ [client 相关方法都是空实现或未导出]
│
└─ scheduler.wasm
   ├─ on_packet_received(meta, payload)
   │  ├─ eventbus.emit(PacketReceived, ...)
   │  ├─ dispatch_to_actions(handle_on_packet_received)
   │  └─ 返回经 actions 处理后的结果
   │
   └─ generate() = 不支持（Client 功能）
```

**Echo Client Wasm 组装**：

```
📦 echo-client.wasm (由 WAC 编排生成)
│
├─ corelibs.wasm
│  ├─ socket API（连接、收发）
│  ├─ timer API
│  ├─ log()
│  └─ 数据结构定义
│
├─ eventbus.wasm
│  ├─ emit_request_sent(seq, timestamp)
│  ├─ emit_response_received(seq, response)
│  └─ on_event_handler()
│
├─ 🎯 actions-executor-client.wasm
│  ├─ handle_on_packet_received(meta, payload)
│  │  └─ ⭐ 业务逻辑 2：接收入站包（可能是响应）
│  │      ├─ 从 payload 提取 sequence/token
│  │      ├─ ✅ 查询已发送请求的记录
│  │      ├─ 验证 token 匹配
│  │      ├─ 计算 RTT（响应时间 - 发送时间）
│  │      ├─ 调用 eventbus.emit(ResponseMatched)
│  │      └─ 更新统计（matched/timeouts）
│  │
│  ├─ handle_generate_request(seq)
│  │  └─ ⭐ 业务逻辑 3：生成出站请求包
│  │      ├─ 构造请求包：[seq(4字节) | test_data]
│  │      ├─ 记录 (seq, 当前时间戳) 到发送队列
│  │      └─ 调用 Host 回调：on_send_packet(payload)
│  │
│  └─ handle_verify_response(response, expected_seq)
│     └─ ⭐ 业务逻辑 4：验证响应包
│         ├─ 从 response 提取 seq
│         ├─ 对比 expected_seq
│         ├─ 返回 true（匹配）或 false（不匹配/超时）
│         └─ 调用 Host 回调：on_recv_packet(timeout_ms)
│
└─ scheduler.wasm
   ├─ generate(count, pps)
   │  ├─ for seq in 0..count {
   │  │    ├─ eventbus.emit(GenerateRequest, seq)
   │  │    ├─ dispatch_to_actions(handle_generate_request, seq)
   │  │    ├─ 等待响应（timeout）
   │  │    ├─ dispatch_to_actions(handle_verify_response)
   │  │    └─ 速率控制：sleep(1_000_000 / pps)
   │  │  }
   │  ├─ eventbus.emit(GenerateComplete)
   │  └─ 返回 GenerateResult { matched, timeouts, rtt_avg, ... }
   │
   └─ on_packet_received() = 不支持（Server 功能）
```

#### 关键差异对比

| 维度 | Echo Server | Echo Client |
|------|------------|-----------|
| **actions-executor 业务逻辑** | `handle_on_packet_received()` 处理入站包并回复 | `handle_generate_request()` 生成请求；`handle_verify_response()` 验证响应 |
| **入口导出** | `scheduler.on_packet_received(meta, payload)` | `scheduler.generate(count, pps)` |
| **核心行为** | 接收 → 回显 → 返回 | 生成 → 发送 → 验证 → 统计 |
| **Host 回调** | 无需 Host 侧回调 | 需要 Host 提供 `on_send_packet()` 和 `on_recv_packet()` 回调 |
| **组件数据流** | scheduler → eventbus → actions(echo) → eventbus → reply | scheduler ← eventbus → actions(generate/verify) ← eventbus ← Host callbacks |

### 1.2 完整的组件依赖关系

#### Echo Server 组件数据流

```
┌─ Host-1 调用：scheduler.on_packet_received(meta, payload)
│
▼
echo-server.wasm (WAC 组装)
│
├─ scheduler 
│  ├─ imports from: eventbus
│  ├─ calls to: actions-executor.handle_on_packet_received()
│  └─ behavior: 协调数据流，触发 eventbus 事件
│
├─ actions-executor-server ⭐ [核心业务层]
│  ├─ imports from: eventbus, corelibs
│  ├─ exports: handle_on_packet_received(meta, payload) → PacketResponse
│  └─ logic: echo 处理（返回相同 payload）
│
├─ eventbus
│  ├─ imports from: corelibs (log)
│  ├─ exports: emit(event), subscribe(handler)
│  └─ role: 组件间事件通路
│
└─ corelibs
   ├─ exports: socket_send(), socket_recv(), log()
   └─ imports: Host WASI 接口（文件系统、网络等）

▲
└─ Host-1 收到返回：response_payload
```

#### Echo Client 组件数据流

```
┌─ Host-2 调用：scheduler.generate(count, pps)
│
▼
echo-client.wasm (WAC 组装)
│
├─ scheduler 
│  ├─ imports from: eventbus, actions-executor
│  ├─ exports: generate(count, pps) → GenerateResult
│  └─ behavior: 循环调用 actions 生成和验证请求
│
├─ actions-executor-client ⭐ [核心业务层]
│  ├─ imports from: eventbus, corelibs, Host callbacks
│  ├─ exports:
│  │  ├─ handle_generate_request(seq) → Vec<u8>  // 生成包
│  │  ├─ handle_on_packet_received(response) → bool  // 验证包
│  │  └─ update_stats(seq, elapsed) → ()  // 统计
│  └─ logic: 请求生成、响应验证、RTT 计算
│
├─ eventbus
│  ├─ imports from: corelibs (log)
│  ├─ exports: emit(event), subscribe(handler)
│  └─ role: 组件间事件通路
│
├─ corelibs
│  ├─ exports: socket API, timer, log()
│  └─ imports: Host WASI 接口
│
└─ Host 侧回调 (wasm imports)
   ├─ on_send_packet(payload) → Host NIC.send()
   └─ on_recv_packet(timeout_ms) → Host NIC.recv()

▲
└─ Host-2 收到返回：GenerateResult { matched, timeouts, rtt_avg, ... }
```

#### 关键区别

**echo-server.wasm**：
- ✅ 内部导出：`on_packet_received(meta, payload) → response`
- ✅ actions-executor：处理**入站包**（Server 逻辑）
- ❌ 不需要 Host callbacks（Host 直接调用导出的 on_packet_received）

**echo-client.wasm**：
- ✅ 内部导出：`generate(count, pps) → result`
- ✅ actions-executor：处理**请求生成和响应验证**（Client 逻辑）
- ✅ 需要 Host callbacks（Wasm 调用 Host 的 on_send_packet / on_recv_packet）
- ✅ 内部包含完整的状态机（发送队列、匹配表、统计）

### 1.3 WAC 编排配置：两个不同的组装方案

#### 核心原则

虽然 echo-server.wac 和 echo-client.wac **使用完全相同的 4 个组件库**（corelibs、eventbus、scheduler、actions-executor），但关键差异在于：

1. **编译时**：actions-executor 编译时包含 **不同的业务逻辑代码**
   - `actions-executor-server`：只有 `handle_on_packet_received()`（echo 逻辑）
   - `actions-executor-client`：有 `handle_generate_request()` 和 `handle_verify_response()`（生成和验证逻辑）

2. **WAC 组装时**：通过不同的 WAC 配置文件组装
   - echo-server.wac → echo-server.wasm（导出 `on_packet_received`）
   - echo-client.wac → echo-client.wasm（导出 `generate`）

#### Echo Server 的 WAC 编排

**文件：`plugins/scheduler/wac/echo-server.wac`**

```plaintext
package scheduler:echo-server;

# 导入 Server 模式的组件（actions-executor 内含 echo 逻辑）
let corelibs = new component "file://../core-libs/target/wasm32-wasip2/release/scheduler_core_libs.wasm";
let eventbus = new component "file://../eventbus/target/wasm32-wasip2/release/scheduler_eventbus.wasm";
let actions = new component "file://../actions-executor/target/wasm32-wasip2/release/scheduler_actions_executor_server.wasm";
let scheduler = new component "file://../scheduler/target/wasm32-wasip2/release/scheduler_server.wasm";

# Server 专用的链接关系
connect eventbus with corelibs;
connect actions with eventbus;
connect actions with corelibs;
connect scheduler with eventbus;
connect scheduler with actions;

# 导出 scheduler 的 on_packet_received 入口
export scheduler;
```

**编译输出**：
```bash
wac plug wac/echo-server.wac -o echo-server.wasm
# 结果：echo-server.wasm
# 导出的方法：on_packet_received(meta, payload) -> response
```

#### Echo Client 的 WAC 编排

**文件：`plugins/scheduler/wac/echo-client.wac`**

```plaintext
package scheduler:echo-client;

# 导入 Client 模式的组件（actions-executor 内含生成和验证逻辑）
let corelibs = new component "file://../core-libs/target/wasm32-wasip2/release/scheduler_core_libs.wasm";
let eventbus = new component "file://../eventbus/target/wasm32-wasip2/release/scheduler_eventbus.wasm";
let actions = new component "file://../actions-executor/target/wasm32-wasip2/release/scheduler_actions_executor_client.wasm";
let scheduler = new component "file://../scheduler/target/wasm32-wasip2/release/scheduler_client.wasm";

# Client 专用的链接关系
connect eventbus with corelibs;
connect actions with eventbus;
connect actions with corelibs;
connect scheduler with eventbus;
connect scheduler with actions;

# 导出 scheduler 的 generate 入口
export scheduler;
```

**编译输出**：
```bash
wac plug wac/echo-client.wac -o echo-client.wasm
# 结果：echo-client.wasm
# 导出的方法：generate(count, pps) -> GenerateResult
# 导入的回调：on_send_packet(payload), on_recv_packet(timeout_ms)
```

#### 关键点对比

| 配置文件 | actions-executor 模式 | scheduler 模式 | 最终导出 | Host 调用方式 |
|---------|-------------------|----------------|---------|-------------|
| **echo-server.wac** | `actions_executor_server` ⭐ | `scheduler_server` ⭐ | `on_packet_received()` | `wasm.call_on_packet_received(meta, payload)` |
| **echo-client.wac** | `actions_executor_client` ⭐ | `scheduler_client` ⭐ | `generate()` | `wasm.call_generate(count, pps)` |

**重点强调**：
- ❌ 不是同一个 actions-executor 用两种模式
- ✅ 是**两个不同的 actions-executor 实现**通过 WAC 分别组装成 Server 和 Client 版本

---

## 1. 架构设计

### 1.1 Phase 1 整体拓扑（立即实现）

```
┌────────────────────────────────────────────────────────────────┐
│ Host-1（Echo Server）                                           │
├────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────────────┐                                       │
│  │ NIC Layer            │  (AF_PACKET / SOCK_DGRAM)            │
│  │ (recv/send)          │                                       │
│  └──────────┬───────────┘                                       │
│             │                                                   │
│  ┌──────────▼───────────┐                                       │
│  │ Packet Decoder       │  (L2/L3/L4 parse)                    │
│  └──────────┬───────────┘                                       │
│             │                                                   │
│  ┌──────────▼───────────────────────────┐                       │
│  │ Echo Server Wasm (WAC 组装)           │                      │
│  │ ├─ scheduler                          │                      │
│  │ ├─ eventbus                           │  Wasm组件                │
│  │ ├─ actions-executor (echo logic)     │  ★ 业务逻辑集中在这
│  │ └─ corelibs                           │                      │
│  └──────────┬───────────────────────────┘                       │
│             │                                                   │
│  ┌──────────▼───────────┐                                       │
│  │ Reply Builder        │  (构造 L2/L3/L4)                      │
│  └──────────┬───────────┘                                       │
│             │                                                   │
│  ┌──────────▼───────────┐                                       │
│  │ NIC TX               │  (send via AF_PACKET)                │
│  └──────────────────────┘                                       │
└────────────────────────────────────────────────────────────────┘
         ▲                           │
         │                           ▼
      UDP/IP                      UDP/IP
         │                           │
┌────────┴───────────────────────────┴─────────────────────────┐
│ Network (veth/physical)                                       │
└────────┬───────────────────────────┬─────────────────────────┘
         │                           ▲
         │                           │
      UDP/IP                      UDP/IP
         │                           │
         ▼                           │
  ┌──────────────────────────────────────┐
  │ Host-2（Echo Client）                │
  ├──────────────────────────────────────┤
  │                                      │
  │  ┌──────────────────────┐            │
  │  │ NIC Layer            │            │
  │  │ (send/recv)          │            │
  │  └──────────┬───────────┘            │
  │             │                        │
  │  ┌──────────▼───────────┐            │
  │  │ Echo Client (Wasm)   │  ⭐ 独立组件
  │  │ - generate(count,pps)│    生成请求
  │  │ - verify(response)   │    验证回复
  │  └──────────┬───────────┘    统计结果
  │             │                │
  │  ┌──────────▼───────────┐    │
  │  │ Packet Builder       │    │
  │  │ (构造请求包)         │    │
  │  └──────────┬───────────┘    │
  │             │                │
  │  ┌──────────▼───────────┐    │
  │  │ Statistics           │◄───┘
  │  │ (matched/timeouts)   │
  │  └──────────────────────┘
  │
  └──────────────────────────────────────┘
```

### 1.2 WAC 组装的组件职责

**Echo Server Wasm（由 WAC 组装）的内部结构：**

| 层级 | 组件 | 职责 | 说明 |
|------|------|------|------|
| **业务层** | actions-executor | ★ 实现 UDP echo 处理逻辑 | 接收 PacketReceived 事件，返回 echo 响应 |
| **协调层** | scheduler | 协调各组件执行流 | 接收 on-packet-received，dispatch 到 actions，发送 emit event |
| **通信层** | eventbus | 消息总线 | 在 scheduler/actions/host 之间传递事件 |
| **基础层** | corelibs | socket API、日志等 | 由 Host 调用的底层接口 |

**Echo Client Wasm（由 WAC 组装）的内部结构：**

| 层级 | 组件 | 职责 | 说明 |
|------|------|------|------|
| **业务层** | actions-executor | ★ 实现请求生成和响应验证 | 生成请求、记录 seq、验证回复 token |
| **协调层** | scheduler | 协调各组件执行流 | 调用 generate()，dispatch 请求/响应任务 |
| **通信层** | eventbus | 消息总线 | Host ↔ Scheduler 的事件传递 |
| **基础层** | corelibs | socket API、日志等 | 由 Host 调用的底层接口 |

**Host 端职责（不在 Wasm 内部）：**

| 层级 | 组件 | 职责 |
|------|------|------|
| **Network I/O** | NIC Layer | AF_PACKET 接收/发送原始包 |
| **Parse** | Packet Decoder | 解析 L2/L3/L4 头，提取 meta + payload |
| **Export** | Wasm Loader | 加载 echo-server.wasm，调用导出接口 |
| **Reply** | Reply Builder | 根据 Wasm 返回的 payload 重新构造包头（交换源目） |

### 1.3 Phase 2 保留设计（多实例编排）

当需要支持多个 Echo Server/Client 实例时，Scheduler 的作用：

| 组件 | 职责 | 说明 |
|------|------|------|
| **Scheduler** | 编排多个实例 | 管理多个 Server/Client 的并发执行 |
| **EventBus** | 事件路由 | Host ↔ Scheduler ↔ Actions 的事件流 |
| **Actions Executor** | 动作执行 | 为每个 echo 请求创建独立的执行上下文 |
| **Core Libs** | 共享库 | Socket、日志、数据结构等 |

### 1.4 Phase 1 完整数据流向（WAC 组装）

**Host-1（Echo Server）收发流程：**

```
网络包
  ↓
NIC.recv() ← Host 程序
  ↓
Packet Decoder.decode() ← Host 程序
  ↓
PacketMeta + PayloadBytes
  ↓
[echo-server.wasm 由 WAC 组装]
  ├─ scheduler.on_packet_received(meta, payload)
  │  ├─ eventbus.emit(PacketReceived)
  │  └─ dispatch_to_actions()
  │
  ├─ actions_executor.handle_echo_action()
  │  ├─ 业务逻辑：echo = payload（不改变内容）
  │  └─ eventbus.emit(EchoComplete)
  │
  ├─ scheduler.on_echo_complete()
  │  └─ return ResponsePayload
  │
  └─ 返回 response_payload
  ↓
Reply Builder.build_reply(meta, response_payload) ← Host 程序
  （交换源目 IP/Port，保留 payload）
  ↓
NIC.send() ← Host 程序
  ↓
网络包返回
```

**Host-2（Echo Client）发送验证流程：**

```
User 启动：generate(count=100, pps=50)
  ↓
[echo-client.wasm 由 WAC 组装]
  ├─ scheduler.generate(count, pps)
  │  ├─ for seq in 0..count {
  │  │    ├─ eventbus.emit(GenerateRequest)
  │  │    ├─ actions_executor.create_request_packet(seq)
  │  │    ├─ Host 回调：on_send_packet(packet)
  │  │    └─ Host 回调：on_recv_packet(timeout)
  │  │       ├─ eventbus.emit(ResponseReceived)
  │  │       └─ actions_executor.verify_token(response, seq)
  │  │           └─ 更新 matched/timeouts 统计
  │  │  }
  │  └─ eventbus.emit(GenerateComplete)
  │
  └─ 返回 GenerateResult { matched, timeouts, rtt_avg, ... }
  ↓
Host 输出结果
```

---

## 2. WAC 编排配置详解

### 2.1 Echo Server WAC 编排

**文件：`plugins/scheduler/wac/echo-server.wac`**

```plaintext
package scheduler:echo-server;

// 导入各子组件
let corelibs = new component "file://../core-libs/target/wasm32-wasip2/release/scheduler_core_libs.wasm";
let eventbus = new component "file://../eventbus/target/wasm32-wasip2/release/scheduler_eventbus.wasm";
let actions = new component "file://../actions-executor/target/wasm32-wasip2/release/scheduler_actions_executor.wasm";
let scheduler = new component "file://../scheduler/target/wasm32-wasip2/release/scheduler.wasm";

// 建立组件间的连接关系
// eventbus 需要使用 corelibs 的日志接口
connect eventbus with corelibs;

// actions-executor 需要使用 corelibs 和 eventbus
connect actions with corelibs;
connect actions with eventbus;

// scheduler 需要协调 eventbus 和 actions
connect scheduler with eventbus;
connect scheduler with actions;

// 导出 scheduler 作为主入口
export scheduler;
```

**生成过程：**

```bash
# 编译各子组件
cd plugins/scheduler
for dir in core-libs eventbus actions-executor scheduler; do
  cd $dir && cargo build --target wasm32-wasip2 --release && cd ..
done

# 使用 WAC 编排
wac plug wac/echo-server.wac -o echo-server.wasm
# 输出：echo-server.wasm
```

### 2.2 Echo Client WAC 编排

**文件：`plugins/scheduler/wac/echo-client.wac`**

```plaintext
package scheduler:echo-client;

let corelibs = new component "file://../core-libs/target/wasm32-wasip2/release/scheduler_core_libs.wasm";
let eventbus = new component "file://../eventbus/target/wasm32-wasip2/release/scheduler_eventbus.wasm";
let actions = new component "file://../actions-executor/target/wasm32-wasip2/release/scheduler_actions_executor.wasm";
let scheduler = new component "file://../scheduler/target/wasm32-wasip2/release/scheduler.wasm";

connect eventbus with corelibs;
connect actions with corelibs;
connect actions with eventbus;
connect scheduler with eventbus;
connect scheduler with actions;

export scheduler;
```

---

## 3. 核心实现：actions-executor 的业务逻辑差异

### 概述

**actions-executor 组件存在两种完全不同的实现**，分别用于 Server 和 Client 场景：

| 实现版本 | 位置 | 导出函数 | 业务逻辑 |
|---------|------|---------|--------|
| **actions-executor-server** | `plugins/scheduler/actions-executor-server/src/lib.rs` | `handle_on_packet_received()` | 接收入站包并回显 |
| **actions-executor-client** | `plugins/scheduler/actions-executor-client/src/lib.rs` | `handle_generate_request()`, `handle_verify_response()` | 生成请求并验证响应 |

### 3.1 Echo Server 的 actions-executor 实现

**文件位置**：`plugins/scheduler/actions-executor-server/src/lib.rs`

```rust
// ★ Server 端业务逻辑：处理入站包并回复

pub fn handle_on_packet_received(
    meta: PacketMeta,
    payload: Vec<u8>,
) -> Result<PacketResponse, String> {
    // 业务逻辑：Echo = 直接返回相同的 payload
    // Host 负责交换源目 IP/Port 和重新构造包头
    
    log_event(&format!(
        "[Server] Received packet from {}:{} -> {}:{}, size={}",
        meta.src_ip, meta.src_port, meta.dst_ip, meta.dst_port, payload.len()
    ));
    
    Ok(PacketResponse {
        payload,  // ★ 核心：payload 原样返回
        forward: true,  // 指示 Host 发送回复
    })
}
```

**工作流**：
```
Host-1 调用：wasm.call_on_packet_received(meta, payload)
  ↓
actions-executor-server.handle_on_packet_received()
  ├─ 验证：payload 不为空
  ├─ 回显：response_payload = payload（原样）
  ├─ 日志：记录处理事件
  └─ 返回：PacketResponse { payload, forward: true }
  ↓
Host-1 接收返回，构造回复包并发送
```

### 3.2 Echo Client 的 actions-executor 实现

**文件位置**：`plugins/scheduler/actions-executor-client/src/lib.rs`

```rust
// ★ Client 端业务逻辑：生成请求并验证响应

// 1. 生成请求包
pub fn handle_generate_request(seq: u32) -> Vec<u8> {
    // 格式：[4 字节 seq (big-endian) | 可选测试数据]
    let mut packet = Vec::new();
    packet.extend_from_slice(&seq.to_be_bytes());
    packet.extend_from_slice(b"Echo test payload");  // 测试数据
    
    log_event(&format!("[Client] Generated request seq={}", seq));
    packet
}

// 2. 验证响应包
pub fn handle_verify_response(response: &[u8], expected_seq: u32) -> bool {
    // 验证：响应的 seq 是否匹配期望值
    if response.len() < 4 {
        log_event(&format!("[Client] Response too short: {} bytes", response.len()));
        return false;
    }
    
    let seq = u32::from_be_bytes([
        response[0],
        response[1],
        response[2],
        response[3],
    ]);
    
    let matched = seq == expected_seq;
    if matched {
        log_event(&format!("[Client] Response verified: seq={}", seq));
    } else {
        log_event(&format!("[Client] Seq mismatch: expected={}, got={}", expected_seq, seq));
    }
    
    matched
}

// 3. 记录请求统计（在 scheduler 中调用）
pub struct RequestStats {
    pub seq: u32,
    pub sent_time: u64,          // 毫秒时间戳
    pub recv_time: Option<u64>,  // 收到响应的时间，None = 超时
}

pub fn update_stats(stats: &mut RequestStats, recv_time: Option<u64>) {
    stats.recv_time = recv_time;
    
    if let Some(recv_t) = recv_time {
        let rtt = recv_t.saturating_sub(stats.sent_time);
        log_event(&format!(
            "[Client] RTT for seq={}: {} ms",
            stats.seq, rtt
        ));
    } else {
        log_event(&format!("[Client] Timeout for seq={}", stats.seq));
    }
}
```

**工作流**：
```
Host-2 调用：wasm.call_generate(count=100, pps=50)
  ↓
scheduler.generate()
  ├─ for seq in 0..100 {
  │    ├─ actions_executor_client.handle_generate_request(seq)
  │    │  └─ 返回：[seq | test_data]
  │    │
  │    ├─ Host 回调：on_send_packet(packet)  【Host 负责发送到网络】
  │    │
  │    ├─ Host 循环接收（timeout=5s）
  │    │  └─ Host 回调：on_recv_packet()  【Host 负责从网络收包】
  │    │
  │    ├─ actions_executor_client.handle_verify_response(response, seq)
  │    │  └─ 返回：true（匹配）或 false（不匹配/超时）
  │    │
  │    ├─ 更新统计：matched++ 或 timeouts++
  │    │
  │    └─ 速率控制：sleep(1_000_000 / 50)  【50 pps = 20ms/packet】
  │  }
  │
  ├─ 计算平均 RTT
  └─ 返回：GenerateResult { matched, timeouts, rtt_avg, ... }
  ↓
Host-2 收到结果，输出统计
```

### 3.3 核心区别总结

| 方面 | Server | Client |
|------|--------|--------|
| **业务函数** | `handle_on_packet_received()` | `handle_generate_request()`, `handle_verify_response()` |
| **入站包处理** | ✅ 必须支持 | ❌ 不需要（或作为响应验证） |
| **出站包生成** | ❌ 不需要 | ✅ 必须支持 |
| **状态机** | 简单（无状态） | 复杂（维护请求队列和匹配表） |
| **Host 回调** | ❌ 无 | ✅ `on_send_packet()`, `on_recv_packet()` |
| **响应验证** | ❌ 无 | ✅ 验证 sequence 匹配 |
| **RTT 计算** | ❌ 无 | ✅ 计算往返时间 |
| **代码复用** | 不与 Client 共享代码 | 不与 Server 共享代码 |

**重点**：
- ❌ actions-executor **不是一个组件**用两种模式
- ✅ actions-executor 是**两个独立的 Rust 项目/Crate**
  - `actions-executor-server/` → 编译成 `actions_executor_server.wasm`
  - `actions-executor-client/` → 编译成 `actions_executor_client.wasm`

---

## 2. Host-1 Echo Server Wasm 实现

### 2.1 Host-1 程序的职责

Host-1 加载 echo-server.wasm 后的执行流程：

```rust
fn main_loop(nic: &Nic, wasm_instance: &Instance) -> Result<()> {
    loop {
        // 1. NIC 接收原始包
        if let Some(buf) = nic.recv_nonblocking() {
            // 2. Host 解析包头
            let meta = decode_packet_meta(&buf)?;
            let payload = extract_payload(&buf)?;
            
            // 3. 调用 Wasm 导出的 scheduler.on_packet_received()
            let response = wasm_instance.call_on_packet_received(meta, payload)?;
            
            // 4. Host 构造回复包（交换源目，使用 Wasm 返回的 payload）
            if response.forward {
                let reply_buf = build_reply_packet(&buf, &response.payload)?;
                nic.send(&reply_buf)?;
            }
        }
    }
}
```

### 2.2 Host-1 Runtime 流程
```
1. 加载 Echo Server Wasm 组件
2. 循环：
   a. NIC.recv() → 原始网络包
   b. Decoder.decode() → PacketMeta + Payload
   c. Wasm on_packet(meta, payload)
   d. 根据 Result 构建回复
   e. Reply Builder.build_packet() → 以太网+IPv4+UDP
   f. NIC.send(reply)
```

### 2.3 关键数据结构

```rust
struct PacketMeta {
    src_mac: [u8; 6],
    dst_mac: [u8; 6],
    src_ip: [u8; 4],      // IPv4
    dst_ip: [u8; 4],
    src_port: u16,        // UDP
    dst_port: u16,
    ether_type: u16,
}

struct PacketResponse {
    payload: Vec<u8>,     // Echo 内容
    // 或 None 代表静默丢弃
}
```

### 2.4 Host-1 主程序流程

在 `src/main.rs` 中添加新的运行模式 `--mode server` 或扩展现有 `--mode net` 的功能：

```rust
fn main_loop(
    nic: &dyn Nic,
    component: &Component,
    config: &ServerConfig,
) -> anyhow::Result<()> {
    let mut stats = Stats::default();
    let mut last_report = Instant::now();

    loop {
        if let Some(buf) = nic.recv_nonblocking() {
            stats.rx_total += 1;
            
            match PacketContext::decode(&buf) {
                Ok(ctx) if ctx.udp_meta.is_some() => {
                    stats.rx_udp += 1;
                    
                    // 调用 Wasm 组件的 on_packet()
                    match call_echo_server_wasm(&component, &ctx) {
                        Ok(Some(response)) => {
                            let reply_buf = build_udp_reply(&ctx, &response)?;
                            nic.send(&reply_buf)?;
                            stats.tx_total += 1;
                        }
                        Ok(None) => stats.rx_dropped += 1,
                        Err(e) => {
                            stats.rx_errors += 1;
                            eprintln!("[!] Wasm error: {}", e);
                        }
                    }
                }
                Ok(_) => { /* 非UDP，忽略 */ }
                Err(e) => {
                    stats.rx_errors += 1;
                }
            }
        } else if nic.poll_readable(config.poll_timeout)? {
            continue;
        } else {
            // 定期统计输出
            if last_report.elapsed() > Duration::from_secs(5) {
                println!("[stats] rx={} rx_udp={} tx={} err={}",
                    stats.rx_total, stats.rx_udp, stats.tx_total, stats.rx_errors);
                last_report = Instant::now();
            }
        }
    }
}
```

### 2.5 启动示例

```bash
# Host-1 启动 Echo Server（加载 Wasm 组件）
./target/debug/Ntx \
  --mode server \
  --iface eth0 \
  --backend afpacket-dgram \
  --port 10001 \
  --component ./plugins/echo/echo-server.wasm
```

---

## 3. Host-2 Echo Client Wasm 实现

### 3.1 Echo Client Wasm 责任
- 导出 WIT 接口：`generate(count, pps) -> GenerateResult`
- 生成指定数量和速率的 echo 请求
- 接收并验证响应（匹配 token/sequence）
- 返回统计信息：matched/timeouts/errors

### 3.2 Echo Client Wasm 数据结构

```rust
struct GenerateRequest {
    count: u32,
    pps: u32,           // packets per second
    dst_ip: [u8; 4],
    dst_port: u16,
}

struct GenerateResult {
    total_sent: u32,
    total_received: u32,
    matched: u32,
    timeouts: u32,
    errors: u32,
    rtt_min_us: u64,
    rtt_max_us: u64,
    rtt_avg_us: u64,
}
```

### 3.3 Host-2 Runtime 流程

```
1. 加载 Echo Client Wasm 组件
2. 调用 Wasm generate(count, pps)
   内部 Wasm 处理：
   a. 生成请求包（带 sequence/token）
   b. 调用 Host 侧发包函数
   c. 循环接收回复
   d. 匹配 sequence/token
   e. 计算 RTT 和统计
   f. 返回 GenerateResult
```

### 3.4 Host-2 主程序流程

```rust
fn main_loop(
    nic: &dyn Nic,
    component: &Component,
    config: &ClientConfig,
) -> anyhow::Result<()> {
    // 调用 Wasm 组件生成请求
    let result = component.call_generate(GenerateRequest {
        count: config.count,
        pps: config.pps,
        dst_ip: config.server_ip,
        dst_port: config.server_port,
    })?;
    
    // Wasm 内部会调用 Host 的发包/收包接口
    // Wasm 返回统计结果
    println!("[result] sent={} matched={} timeouts={} avg_rtt={}us",
        result.total_sent, result.matched, result.timeouts, result.rtt_avg_us);
    
    Ok(())
}
```

### 3.5 启动示例

```bash
# Host-2 启动 Echo Client（加载 Wasm 组件）
./target/debug/Ntx \
  --mode client \
  --iface eth1 \
  --backend afpacket-dgram \
  --server-ip 10.0.0.1 \
  --server-port 10001 \
  --component ./plugins/echo/echo-client.wasm \
  --count 1000 \
  --pps 100
```

---

## 4. Phase 1 Wasm 组件实现

### 4.1 Echo Server Wasm 组件

#### 4.1.1 WIT 定义

**文件：`plugins/echo/echo-server/wit/world.wit`**

```wit
package echo:server;

interface server {
  record packet-meta {
    src-ip: list<u8>,          // IPv4 or IPv6
    dst-ip: list<u8>,
    src-port: u16,
    dst-port: u16,
    ether-type: u16,
    timestamp: u64,             // Unix timestamp in milliseconds
  }
  
  record packet-response {
    payload: list<u8>,
    forward: bool,              // true = send reply, false = drop
  }
  
  on-packet: func(meta: packet-meta, payload: list<u8>) 
    -> result<packet-response, string>;
}

world echo-server {
  export server;
}
```

#### 4.1.2 Rust 实现

**文件：`plugins/echo/echo-server/src/lib.rs`**

```rust
use wit_bindgen::generate!();

generate!();

export!(Component);

struct Component;

impl Guest for Component {
    fn on_packet(
        meta: ServerPacketMeta,
        payload: Vec<u8>,
    ) -> Result<PacketResponse, String> {
        // Echo 逻辑：直接返回相同 payload，Host 负责交换源目
        Ok(PacketResponse {
            payload,
            forward: true,
        })
    }
}
```

#### 4.1.3 编译步骤

```bash
cd plugins/echo/echo-server
cargo build --target wasm32-wasip2
# 输出：target/wasm32-wasip2/debug/echo_server.wasm
```

### 4.2 Echo Client Wasm 组件

#### 4.2.1 WIT 定义

**文件：`plugins/echo/echo-client/wit/world.wit`**

```wit
package echo:client;

interface client {
  record generate-config {
    count: u32,
    pps: u32,                   // packets per second
    dst-ip: list<u8>,
    dst-port: u16,
  }
  
  record generate-result {
    total-sent: u32,
    total-received: u32,
    matched: u32,
    timeouts: u32,
    errors: u32,
    rtt-min-us: u64,
    rtt-max-us: u64,
    rtt-avg-us: u64,
  }
  
  generate: func(config: generate-config) -> result<generate-result, string>;
  
  // 回调：Host 侧提供的函数，Wasm 调用来发包和收包
  on-send-packet: func(payload: list<u8>) -> result<(), string>;
  on-recv-packet: func(timeout-ms: u32) -> result<option<list<u8>>, string>;
}

world echo-client {
  export client;
  import on-send-packet;
  import on-recv-packet;
}
```

#### 4.2.2 Rust 实现

**文件：`plugins/echo/echo-client/src/lib.rs`**

```rust
use wit_bindgen::generate!();

generate!();

export!(Component);

struct Component;

impl Guest for Component {
    fn generate(config: GenerateConfig) -> Result<GenerateResult, String> {
        let mut result = GenerateResult {
            total_sent: 0,
            total_received: 0,
            matched: 0,
            timeouts: 0,
            errors: 0,
            rtt_min_us: u64::MAX,
            rtt_max_us: 0,
            rtt_avg_us: 0,
        };
        
        let mut rtt_sum: u64 = 0;
        
        for seq in 0..config.count {
            // 构造请求包（payload 包含 sequence token）
            let request = create_echo_request(seq);
            
            // Host 侧发包
            if let Err(e) = on_send_packet(&request) {
                result.errors += 1;
                eprintln!("Send error: {}", e);
                continue;
            }
            result.total_sent += 1;
            
            let start_time = std::time::Instant::now();
            
            // 尝试接收回复（带 timeout）
            match on_recv_packet(5000) {  // 5 秒 timeout
                Ok(Some(response)) => {
                    let elapsed = start_time.elapsed().as_micros() as u64;
                    
                    result.total_received += 1;
                    
                    // 验证 sequence token
                    if verify_token(&response, seq) {
                        result.matched += 1;
                        
                        // 更新 RTT 统计
                        result.rtt_min_us = result.rtt_min_us.min(elapsed);
                        result.rtt_max_us = result.rtt_max_us.max(elapsed);
                        rtt_sum += elapsed;
                    }
                }
                Ok(None) => {
                    result.timeouts += 1;
                }
                Err(e) => {
                    result.errors += 1;
                    eprintln!("Recv error: {}", e);
                }
            }
            
            // 速率控制：pps (packets per second)
            let interval_us = 1_000_000 / config.pps as u64;
            std::thread::sleep(std::time::Duration::from_micros(interval_us));
        }
        
        // 计算平均 RTT
        if result.matched > 0 {
            result.rtt_avg_us = rtt_sum / result.matched as u64;
        }
        
        Ok(result)
    }
}

// 辅助函数
fn create_echo_request(seq: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    // 格式：[4 bytes seq][rest is data]
    payload.extend_from_slice(&seq.to_be_bytes());
    payload.extend_from_slice(b"Echo test payload");
    payload
}

fn verify_token(response: &[u8], expected_seq: u32) -> bool {
    if response.len() < 4 {
        return false;
    }
    let seq = u32::from_be_bytes([response[0], response[1], response[2], response[3]]);
    seq == expected_seq
}
```

#### 4.2.3 编译步骤

```bash
cd plugins/echo/echo-client
cargo build --target wasm32-wasip2
# 输出：target/wasm32-wasip2/debug/echo_client.wasm
```

### 4.3 Phase 1 WAC 策略

对于 Phase 1，**不使用 WAC 编排**。理由：

1. 两个组件各自独立，运行在不同 Host
2. Host 程序直接加载各自的 .wasm 文件
3. 简化部署和调试

#### 文件结构

```
plugins/echo/
├── echo-server/
│   ├── Cargo.toml
│   ├── wit/
│   │   └── world.wit
│   └── src/
│       └── lib.rs
├── echo-client/
│   ├── Cargo.toml
│   ├── wit/
│   │   └── world.wit
│   └── src/
│       └── lib.rs
└── build.sh
```

#### build.sh

```bash
#!/bin/bash
set -e

cd "$(dirname "$0")"

echo "[*] Building echo-server..."
cd echo-server
cargo build --target wasm32-wasip2 "$@"
cd ..

echo "[*] Building echo-client..."
cd echo-client
cargo build --target wasm32-wasip2 "$@"
cd ..

echo "[+] Build complete"
echo "    Server: echo-server/target/wasm32-wasip2/debug/echo_server.wasm"
echo "    Client: echo-client/target/wasm32-wasip2/debug/echo_client.wasm"
```

---

## 5. Host 集成

### 5.1 Host-1 Wasm 加载和调用

在 `src/main.rs` 中添加 `--mode server` 支持：

```rust
// 伪代码概览

fn server_mode(args: &Args) -> anyhow::Result<()> {
    // 加载 Echo Server Wasm
    let store = wasmtime::Store::new(&engine, store_data);
    let module = wasmtime::Module::from_file(&engine, &args.wasm_path)?;
    let mut linker = wasmtime::Linker::new(&engine);
    let instance = linker.instantiate(&mut store, &module)?;
    
    // 获取导出的 on_packet 函数
    let on_packet = instance
        .get_typed_func::<(PacketMeta, Vec<u8>), PacketResponse>(&mut store, "on-packet")?;
    
    // 主循环
    let nic = Nic::new(&args)?;
    loop {
        if let Some(buf) = nic.recv_nonblocking() {
            let ctx = PacketContext::decode(&buf)?;
            if let Some(meta) = ctx.udp_meta {
                // 调用 Wasm
                match on_packet.call(&mut store, (meta, ctx.payload.to_vec())) {
                    Ok(response) => {
                        let reply = build_reply(&ctx, &response)?;
                        nic.send(&reply)?;
                    }
                    Err(e) => eprintln!("Wasm error: {}", e),
                }
            }
        }
    }
}
```

### 5.2 Host-2 Wasm 加载和调用

在 `src/main.rs` 中添加 `--mode client` 支持：

```rust
fn client_mode(args: &Args) -> anyhow::Result<()> {
    // 加载 Echo Client Wasm  
    let store = wasmtime::Store::new(&engine, store_data);
    let module = wasmtime::Module::from_file(&engine, &args.wasm_path)?;
    
    // 链接 Host 侧的回调函数
    let mut linker = wasmtime::Linker::new(&engine);
    linker.func_wrap("", "on-send-packet", |payload: Vec<u8>| {
        // Host 实现：通过 NIC 发送包
        // ...
    })?;
    linker.func_wrap("", "on-recv-packet", |timeout_ms: u32| -> Option<Vec<u8>> {
        // Host 实现：从 NIC 接收包并返回
        // ...
    })?;
    
    let instance = linker.instantiate(&mut store, &module)?;
    
    // 获取导出的 generate 函数
    let generate = instance
        .get_typed_func::<GenerateConfig, GenerateResult>(&mut store, "generate")?;
    
    // 调用 generate
    let config = GenerateConfig {
        count: args.count,
        pps: args.pps,
        dst_ip: parse_ip(&args.server_ip),
        dst_port: args.server_port,
    };
    
    let result = generate.call(&mut store, config)?;
    println!("[result] sent={} matched={} avg_rtt={}us", 
        result.total_sent, result.matched, result.rtt_avg_us);
    
    Ok(())
}
```

---

## 6. 端到端流程

### 6.1 初始化阶段

```
[Host-1] 启动 → 加载 Echo Server Wasm → 准备接收
[Host-2] 启动 → 加载 Echo Client Wasm → 准备发送
```

### 6.2 执行阶段

```
[Host-2] generate(count=100, pps=50)
  ↓
[Host-2 Wasm] 循环生成请求
  ↓
[Host-2] 调用 on-send-packet() callback → NIC.send()
  ↓
[网络] 包传输
  ↓
[Host-1] NIC.recv() → Decoder.decode() → PacketMeta + Payload
  ↓
[Host-1] 调用 Wasm on_packet(meta, payload)
  ↓
[Host-1 Wasm] Echo 处理：返回相同 payload
  ↓
[Host-1] Reply Builder → NIC.send()
  ↓
[网络] 回复包传输
  ↓
[Host-2] NIC.recv() → response_parser
  ↓
[Host-2 Wasm] verify() 检查 sequence token 匹配
  ↓
[Host-2 Wasm] 统计 matched/timeouts，计算 RTT
  ↓
[Host-2] generate() 返回 GenerateResult
```

### 6.3 结果输出

```
[Host-2] 输出
  sent=100
  matched=99
  timeouts=1
  avg_rtt=245us
  min_rtt=234us
  max_rtt=312us
```

---

## 7. 编译和部署

### 7.1 编译 Wasm 组件

```bash
# 编译 Echo Server
cd plugins/echo/echo-server
cargo build --target wasm32-wasip2 --release

# 编译 Echo Client  
cd ../echo-client
cargo build --target wasm32-wasip2 --release

# 验证输出
ls -la */target/wasm32-wasip2/release/*.wasm
```

### 7.2 编译 Host 主程序

```bash
# 在项目根目录
cargo build --release

# 输出：target/release/Ntx
```

### 7.3 启动 Host-1（Server）

```bash
# Host-1 启动 Echo Server
sudo ./scripts/ntxns1.sh \
  timeout 60 \
  ./target/release/Ntx \
  --mode server \
  --iface eth0 \
  --backend afpacket-dgram \
  --port 10001 \
  --component ./plugins/echo/echo-server/target/wasm32-wasip2/release/echo_server.wasm
```

**预期输出**：
```
[*] Loaded Echo Server Wasm
[*] Listening on 10.0.0.1:10001
[stats] rx=0 tx=0 err=0
```

### 7.4 启动 Host-2（Client）

```bash
# Host-2 启动 Echo Client
sudo ./scripts/ntxns2.sh \
  ./target/release/Ntx \
  --mode client \
  --iface eth1 \
  --backend afpacket-dgram \
  --server-ip 10.0.0.1 \
  --server-port 10001 \
  --component ./plugins/echo/echo-client/target/wasm32-wasip2/release/echo_client.wasm \
  --count 1000 \
  --pps 100
```

**预期输出**：
```
[result] sent=1000 matched=998 timeouts=2 avg_rtt=245us
```

---

## 8. 文件结构总结

```
Ntx/
├── src/
│   └── main.rs                    # 扩展以支持 --mode server/client
├── plugins/
│   └── echo/
│       ├── echo-server/           # ★ Echo Server Wasm 项目
│       │   ├── Cargo.toml
│       │   ├── wit/
│       │   │   └── world.wit
│       │   └── src/
│       │       └── lib.rs
│       ├── echo-client/           # ★ Echo Client Wasm 项目
│       │   ├── Cargo.toml
│       │   ├── wit/
│       │   │   └── world.wit
│       │   └── src/
│       │       └── lib.rs
│       └── build.sh
├── scripts/
│   ├── ntx-veth-up.sh             # veth 拓扑
│   ├── ntxns1.sh                  # netns1 包装
│   ├── ntxns2.sh                  # netns2 包装
│   └── ntx-e2e-echo.sh            # ★ Echo 场景自动化脚本
└── docs/
    ├── SCENARIO_ECHO_DESIGN.md    # ★ 本文档（Phase 1 设计）
    ├── IMPLEMENTATION_GUIDE.md    # ★ 实现指南（Phase 1 步骤）
    └── ECHO_QUICKSTART.md         # ★ 快速开始
```

### Phase 2 保留结构

```
plugins/
└── scheduler/
    ├── scheduler/                 # Scheduler 组件（Phase 2）
    ├── eventbus/                  # EventBus 组件（Phase 2）
    ├── core-libs/                 # Core Libs 组件（Phase 2）
    ├── actions-executor/          # Actions Executor 组件（Phase 2）
    └── wac/
        └── echo_scenario.wac      # Phase 2 WAC 编排配置
```

---

## 9. 快速检查清单

### 编译

- [ ] `cd plugins/echo && ./build.sh` 成功生成 .wasm
- [ ] `cargo build --release` 主程序编译成功
- [ ] `file` 命令验证 .wasm 为有效格式

### 部署

- [ ] Host-1 可加载 echo-server.wasm
- [ ] Host-2 可加载 echo-client.wasm
- [ ] veth 网络拓扑正确配置

### 测试

- [ ] Host-1 能接收 UDP 包并调用 Wasm
- [ ] Host-2 能生成 UDP 包并验证回复
- [ ] 最终统计数据显示 matched > 0

---

## 10. 常见问题

### Q1: Wasm 编译失败

**A:** 确保已安装 `wasm32-wasip2` target：

```bash
rustup target add wasm32-wasip2
```

### Q2: Host 加载 Wasm 失败

**A:** 检查路径和文件权限：

```bash
ls -la plugins/echo/*/target/wasm32-wasip2/debug/*.wasm
```

### Q3: 没有收到 Echo 回复

**A:** 检查：
- veth 拓扑是否正确：`ip link show`
- IP 地址是否配置：`ip addr show`
- Wasm 是否被调用（可加入日志）

### Q4: RTT 统计为 0

**A:** 确保时间精度足够，检查 `Instant::now()` 的时间分辨率。

---

**文档版本：2.0 | 更新时间：2024-12-14 | 设计阶段：Phase 1**
| Phase 1 | 实现 Host-1 on-udp 调用 | 1-2h |
| Phase 2 | 完成 Guest 导出和 WAC 组合 | 1-2h |
| Phase 3 | 集成 EventBus 事件分发 | 2-3h |
| Phase 4 | 支持 Scheduler 任务调度 | 2-3h |
| Phase 5 | 性能测试和优化 | 2-4h |
| Phase 6 | 文档完善和示例补充 | 1-2h |

---

## 11. 常见问题 & 故障排除

### Q1: Guest 返回值解析出错
**A:** 检查 Wasmtime 版本和 `Val` 编码。参见 `src/guest_packet_val.rs`。

### Q2: WAC 组合失败
**A:** 确保所有子组件已编译为 wasm32-wasip2，且 WIT 文件一致。

### Q3: 没有看到 UDP 包
**A:** 检查 veth 拓扑、IP 地址、端口配置。运行 `tcpdump` 验证网络接口。

### Q4: 性能不达预期
**A:** 检查 NIC 后端选择（afpacket-dgram vs afpacket），运行 `NTX_DEBUG=1` 查看详细日志。

---

**文档版本：1.0 | 更新时间：2024-12-14**
