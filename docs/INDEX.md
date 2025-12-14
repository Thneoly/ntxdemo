# 📚 NIC-HOST-Guest Echo 场景 - 完整文档索引

## 快速导航

### 🏃 我想 5 分钟快速了解
→ 阅读：`docs/ECHO_QUICKSTART.md`

### 👨‍💻 我想实现代码
→ 阅读：`docs/IMPLEMENTATION_GUIDE.md`

### 🏗️ 我想深入理解架构
→ 阅读：`docs/SCENARIO_ECHO_DESIGN.md`

### 📋 我想追踪项目进度
→ 查看：`docs/ECHO_CHECKLIST.md`

### 📊 我想看整体交付总结
→ 查看：`docs/DELIVERY_SUMMARY.md`

---

## 📁 完整文件清单

### 📖 核心设计文档

| 文件 | 大小 | 章节 | 适用对象 | 时间 |
|------|------|------|---------|------|
| **ECHO_QUICKSTART.md** | 200 L | 8 | 所有人（快速入门） | 5 min |
| **SCENARIO_ECHO_DESIGN.md** | 1000 L | 11 | 架构师、系统设计师 | 30 min |
| **IMPLEMENTATION_GUIDE.md** | 800 L | 12 | 开发工程师 | 1-2 h |
| **ECHO_CHECKLIST.md** | 300 L | 8 | 项目经理、技术主管 | 15 min |
| **DELIVERY_SUMMARY.md** | 400 L | - | 所有利益相关者 | 10 min |

### 🔧 可执行脚本

| 文件 | 用途 | 说明 |
|------|------|------|
| `scripts/ntx-e2e-echo.sh` | E2E 自动化测试 | 一键启动完整场景，包括网络拓扑、日志收集、验证 |
| `scripts/ntx-veth-up.sh` | 网络拓扑设置 | 建立 veth + netns，已有（Echo 场景使用） |
| `scripts/ntxns1.sh` | Host-1 包装器 | 在 netns1 中运行命令，已有（Echo 场景使用） |
| `scripts/ntxns2.sh` | Host-2 包装器 | 在 netns2 中运行命令，已有（Echo 场景使用） |

### ⚙️ 配置文件

| 文件 | 用途 | 说明 |
|------|------|------|
| `plugins/scheduler/wac/echo_scenario.wac` | WAC 组件配置 | 新创建，用于组装各子组件 |

### 📝 代码参考

| 文件 | 用途 | 说明 |
|------|------|------|
| `src/main.rs` | 主程序 | ⏳ Phase 2：需要集成 Guest 调用 |
| `src/guest_packet_val.rs` | Guest 返回值解析 | ✅ 已有，处理 Wasmtime Val 编码 |
| `examples/traffic-send.rs` | 流量生成器 | ✅ 已完成，支持 RR 验证 |
| `network/nic/afpacket.rs` | NIC 实现 | ✅ 已有，支持 raw 和 cooked |
| `plugins/scheduler/scheduler/src/lib.rs` | Guest 实现 | ⏳ Phase 2：需要实现 on-udp 导出 |
| `plugins/scheduler/scheduler/wit/packet.wit` | WIT 接口 | ⏳ Phase 2：需要创建 |

---

## 🎓 学习路径

### 路径 A：从零开始快速上手（推荐新手）

```
1. ECHO_QUICKSTART.md (5 min)
   ↓ 理解基本概念和命令
2. README.md Echo 章节 (5 min)
   ↓ 了解项目结构
3. SCENARIO_ECHO_DESIGN.md 架构部分 (15 min)
   ↓ 理解整体设计
4. 运行自动化脚本 (5 min)
   sudo ./scripts/ntx-e2e-echo.sh
   ↓ 看到实际运行效果
5. IMPLEMENTATION_GUIDE.md Phase 2 (1-2 h)
   ↓ 开始写代码
```

### 路径 B：深度理解架构（适合架构师）

```
1. SCENARIO_ECHO_DESIGN.md (30 min)
   ↓ 全面理解设计
2. IMPLEMENTATION_GUIDE.md (1 h)
   ↓ 了解实现细节
3. ECHO_CHECKLIST.md (10 min)
   ↓ 了解工作流和扩展点
4. 运行 E2E 脚本验证 (5 min)
```

### 路径 C：快速进行代码实现（适合开发者）

```
1. ECHO_QUICKSTART.md 快速参考部分 (5 min)
2. IMPLEMENTATION_GUIDE.md Phase 2 (1-2 h)
   ↓ 详细代码指导
3. 按步骤修改源代码
4. 运行 E2E 脚本验证
```

### 路径 D：项目管理与跟踪

```
1. ECHO_QUICKSTART.md (5 min)
2. ECHO_CHECKLIST.md (15 min)
   ↓ 了解四个 Phase 和预计时间
3. DELIVERY_SUMMARY.md (10 min)
   ↓ 了解当前状态和下一步
4. 定期参考 Phase 列表追踪进度
```

---

## 📌 关键概念速记

### 架构三层

```
Host-1 (Server)      veth      Host-2 (Client)
     ↓                ↔              ↓
  NIC RX          UDP/IP        RR Verify
     ↓                              ↑
  Decoder                      traffic-send
     ↓
  Guest Call
     ↓
  Scheduler+CoreLibs+EventBus+Actions (Wasm)
     ↓
  Task Execution
     ↓
  Reply Builder
     ↓
  NIC TX → UDP/IP → veth
```

### 数据流三阶段

| 阶段 | 操作 | 组件 |
|------|------|------|
| **RX** | 接收网络包、解析 L2/L3/L4 | NIC + Decoder |
| **处理** | 调用 Guest on-udp、执行业务逻辑 | Host + Guest |
| **TX** | 构造回复包、发送 | Reply Builder + NIC |

### 四个实现阶段

| Phase | 工作 | 预计时间 | 状态 |
|-------|------|---------|------|
| 1 | 架构设计 + 文档 | 6-8 h | ✅ 完成 |
| 2 | 代码实现 | 3-4 h | 🔄 准备中 |
| 3 | 测试验证 | 2-3 h | ⏳ 计划中 |
| 4 | 功能扩展 | 4-6 h | ⏳ 计划中 |

---

## 🔧 常用命令速查

### 编译

```bash
# 主程序 + traffic-send
cargo build && cargo build --examples

# Guest 组件
cd plugins/scheduler/scheduler
cargo build --target wasm32-wasip2

# WAC 组合
cd ../wac
wac plug echo_scenario.wac -o echo_composed.wasm
```

### 运行

```bash
# 一键 E2E 测试（推荐）
sudo ./scripts/ntx-e2e-echo.sh --tcpdump

# 手动分步
sudo ./scripts/ntx-veth-up.sh
timeout 30 sudo ./scripts/ntxns1.sh ./target/debug/Ntx --mode net ...
sudo ./scripts/ntxns2.sh ./target/debug/examples/traffic-send ...
```

### 验证

```bash
# 查看日志
tail -f /tmp/ntx-echo-host.log
tail -f /tmp/ntx-echo-client.log

# 查看抓包
tcpdump -r /tmp/ntx-echo-tcpdump.pcap -X
```

---

## ❓ 常见问题速答

### Q1: 我应该从哪个文档开始？
**A:** 如果只有 5 分钟，读 `ECHO_QUICKSTART.md`；如果要实现代码，读 `IMPLEMENTATION_GUIDE.md`

### Q2: 代码示例在哪里？
**A:** `IMPLEMENTATION_GUIDE.md` 中 Phase 1-4 的每个步骤都有完整代码示例

### Q3: 如何运行整个场景？
**A:** `sudo ./scripts/ntx-e2e-echo.sh --tcpdump`（一条命令搞定）

### Q4: 如何追踪项目进度？
**A:** 参考 `ECHO_CHECKLIST.md` 中的实现清单和工作流

### Q5: 遇到问题怎么办？
**A:** 查看各文档的"常见问题"部分，或参考 `ECHO_CHECKLIST.md` 的"常见陷阱与排查"

---

## 📊 文档使用统计

| 使用场景 | 推荐文档 | 阅读时间 |
|---------|---------|---------|
| 快速了解 | ECHO_QUICKSTART.md | 5 min |
| 理解架构 | SCENARIO_ECHO_DESIGN.md | 30 min |
| 实现代码 | IMPLEMENTATION_GUIDE.md | 1-2 h |
| 追踪进度 | ECHO_CHECKLIST.md | 10 min |
| 查看现状 | DELIVERY_SUMMARY.md | 10 min |
| 命令速查 | ECHO_QUICKSTART.md "命令速查" | 2 min |
| 问题排查 | 任何文档的 "常见问题" | 5-10 min |

---

## 🎯 文档特色

### 🎬 完整代码示例
每个实现步骤都有：
- ✅ 完整的 Rust/WIT/WAC 代码
- ✅ 关键注释和说明
- ✅ 编译/运行命令

### 📊 详尽的表格和图表
包括：
- ✅ 架构拓扑图
- ✅ 数据流向图
- ✅ 组件职责矩阵
- ✅ 参考对比表

### ⚡ 快速参考卡片
- ✅ 命令速查表
- ✅ 常见问题速答
- ✅ 文件树速览
- ✅ 验证清单

### 💡 故障排除指南
- ✅ 编译错误诊断
- ✅ 运行时错误排查
- ✅ 常见陷阱说明
- ✅ 解决方案步骤

---

## 📞 反馈与更新

所有文档均位于 `/home/cc/Desktop/code/GitHub/Ntx/docs/`

文档更新频率：
- Phase 2 开始时更新代码示例
- Phase 3 收集测试数据后更新性能指标
- 发现问题时实时更新 Q&A 部分

---

## 🚀 建议使用流程

```
Day 1:
  ├─ 阅读 ECHO_QUICKSTART.md (快速理解)
  └─ 运行 E2E 脚本 (看到实际效果)

Day 2-3:
  ├─ 阅读 IMPLEMENTATION_GUIDE.md (深入学习)
  └─ 按步骤实现 Phase 2

Day 4-5:
  ├─ 测试和调试
  └─ 运行 E2E 脚本验证

Day 6-7:
  ├─ 集成 EventBus/Scheduler (Phase 4)
  └─ 性能优化
```

---

**文档版本**: 1.0 | **更新时间**: 2024-12-14 | **状态**: Phase 1 完成 ✅
