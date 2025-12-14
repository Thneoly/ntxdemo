# Echo 场景实现清单与文档索引

## 📋 文档导航

| 文档 | 描述 | 对象 |
|------|------|------|
| **SCENARIO_ECHO_DESIGN.md** | 完整的架构设计、数据流、组件职责 | 系统设计师 |
| **IMPLEMENTATION_GUIDE.md** | Host-1 集成、Guest 导出、WAC 组装的详细代码步骤 | 开发工程师 |
| **本文档** | 实现清单、文件清单、快速参考 | 项目经理/开发者 |

---

## 🎯 实现清单

### Phase 1: 架构与设计
- [x] NIC-HOST-Guest 整体拓扑设计
- [x] 数据流向图（RX/处理/TX）
- [x] 组件职责矩阵
- [x] Host-1 Server 主循环设计
- [x] Host-2 Client 设计
- [x] Guest Wasm 组件接口设计

**输出**: `docs/SCENARIO_ECHO_DESIGN.md`

### Phase 2: 代码实现
- [ ] **Host-1 集成**（`src/main.rs` 修改）
  - [ ] 添加 `--component` CLI 参数
  - [ ] 加载 Guest Wasm 组件
  - [ ] 实现 `invoke_guest_on_udp()` 函数
  - [ ] 集成主循环（接收 → Guest 调用 → 回复）
  - [ ] 添加统计信息输出

- [ ] **Guest 导出**（`plugins/scheduler/scheduler/`）
  - [ ] 编写 `wit/packet.wit` 接口定义
  - [ ] 在 `src/lib.rs` 实现 `on-udp` 导出
  - [ ] 实现 `PacketMeta` 和 `UdpResponse` 结构体
  - [ ] 实现任务解析和执行逻辑
  - [ ] 编译为 wasm32-wasip2 目标

- [ ] **WAC 组件组装**
  - [ ] 创建 `plugins/scheduler/wac/echo_scenario.wac`
  - [ ] 使用 `wac plug` 生成 `echo_composed.wasm`
  - [ ] 验证组件导出接口

**输出**: `docs/IMPLEMENTATION_GUIDE.md` + 修改的源代码

### Phase 3: 测试与验证
- [x] 创建 E2E 自动化脚本 `scripts/ntx-e2e-echo.sh`
- [ ] 运行单步测试验证各组件
- [ ] 运行完整的端到端测试
- [ ] 验证统计信息准确性
- [ ] 使用 tcpdump 验证网络数据

**输出**: 成功的 E2E 测试运行，exit code 0，matched > 0

### Phase 4: 文档完善
- [x] 主 README 更新 Echo 场景部分
- [x] 创建 `docs/SCENARIO_ECHO_DESIGN.md`
- [x] 创建 `docs/IMPLEMENTATION_GUIDE.md`
- [ ] 补充 Q&A 和故障排除指南
- [ ] 性能基准数据收集

---

## 📁 文件清单

### 已创建/修改的文件

```
Ntx/
├── docs/
│   ├── SCENARIO_ECHO_DESIGN.md          ✅ 已创建 - 架构设计
│   └── IMPLEMENTATION_GUIDE.md           ✅ 已创建 - 实现指南
├── scripts/
│   └── ntx-e2e-echo.sh                  ✅ 已创建 - E2E 自动化
├── plugins/scheduler/wac/
│   └── echo_scenario.wac                ✅ 已创建 - WAC 配置
└── README.md                            ✅ 已更新 - Echo 场景章节
```

### 待修改的文件

```
Ntx/
├── src/
│   └── main.rs                          ⏳ 需要修改 - Guest 调用集成
└── plugins/scheduler/scheduler/
    ├── Cargo.toml                       ⏳ 需要修改 - 组件元数据
    ├── src/lib.rs                       ⏳ 需要修改 - on-udp 实现
    └── wit/
        └── packet.wit                   ⏳ 需要创建 - 接口定义
```

---

## 🚀 快速开始（开发阶段）

### Step 1: 编译
```bash
cd /home/cc/Desktop/code/GitHub/Ntx

# 编译主程序和 traffic-send
cargo build
cargo build --examples

# 编译 Guest 组件（在完成代码修改后）
cd plugins/scheduler/scheduler
cargo build --target wasm32-wasip2

# 生成 WAC 组合组件
cd ../wac
wac plug echo_scenario.wac -o echo_composed.wasm
```

### Step 2: 运行
```bash
# 一键运行完整 E2E 测试
sudo ./scripts/ntx-e2e-echo.sh --tcpdump
```

### Step 3: 验证
```
预期输出：
- Host-1: [stats] rx_udp=20 tx_replies=20
- Host-2: [final] sent=20 matched=20 timeouts=0
- Exit code: 0
```

---

## 📊 关键指标

| 指标 | 目标 | 验证方式 |
|------|------|---------|
| **RR 匹配率** | ≥ 95% | `traffic-send` output: matched/sent |
| **rx_udp 计数** | > 0 | Host-1 stats |
| **tx_replies 计数** | > 0 | Host-1 stats |
| **Guest 调用成功** | 100% | Host-1 logs: no errors |
| **E2E 脚本 exit code** | 0 | `echo $?` |

---

## 🔧 集成工作流

### 当前状态
- ✅ 架构设计完成
- ✅ 文档框架完成
- ✅ E2E 自动化脚本完成
- ⏳ 代码实现（Phase 2）

### 下一步（开发阶段）

**1. 本周内完成 Host-1 集成**
   - 阅读 `docs/IMPLEMENTATION_GUIDE.md`
   - 按 Step 1.1 ~ Step 1.4 修改 `src/main.rs`
   - 编译验证无编译错误
   
**2. 实现 Guest on-udp 导出**
   - 创建 `plugins/scheduler/scheduler/wit/packet.wit`
   - 按 Step 2.1 ~ Step 2.3 实现 Guest 导出
   - 编译为 wasm32-wasip2
   
**3. 生成 WAC 组合组件**
   - 使用 `wac plug` 生成 `echo_composed.wasm`
   - 验证导出接口
   
**4. 运行端到端测试**
   - 执行 `sudo ./scripts/ntx-e2e-echo.sh --tcpdump`
   - 收集日志并验证结果
   
**5. 性能基准测试**
   - 不同 PPS 下的吞吐量
   - 延迟分布
   - 错误率

---

## 📌 常见陷阱与排查

### 编译错误

| 错误 | 原因 | 解决方案 |
|------|------|---------|
| `can't find on-udp` | WIT 文件未定义或路径错误 | 检查 `packet.wit` 和 `Cargo.toml` metadata |
| `Val::Result not found` | Wasmtime 版本问题 | 更新 Wasmtime：`cargo install --locked wasmtime-cli@14.0.0` |
| `wac plug` 失败 | 子组件路径或格式错误 | 检查 .wac 文件中的路径，确保 wasm 文件存在 |

### 运行时错误

| 现象 | 原因 | 排查步骤 |
|------|------|---------|
| Host-1 启动但无 UDP 包 | NIC 配置或后端不匹配 | 检查 --iface 和 --backend |
| Host-2 无 matched | Guest 没有返回响应 | 启用 `NTX_DEBUG=1` 查看 Guest 错误 |
| tcpdump 看不到包 | veth 拓扑未建立 | 运行 `sudo ./scripts/ntx-veth-up.sh` 验证 |
| Exit code 非零 | 脚本超时或测试失败 | 查看 `/tmp/ntx-echo-host.log` 和 `/tmp/ntx-echo-client.log` |

---

## 🎓 参考资源

### 组件和工具
- **WAC 文档**: https://github.com/bytecodealliance/wac
- **Component Model**: https://github.com/WebAssembly/component-model/blob/main/design/mvp/WIT.md
- **Wasmtime**: https://docs.wasmtime.dev/

### 既有实现参考
- `src/guest_packet_val.rs` - Guest 返回值解析
- `network/nic/afpacket.rs` - NIC 实现
- `examples/traffic-send.rs` - 流量生成和 RR 匹配
- `plugins/scheduler/` - Scheduler 组件结构

---

## 📞 支持与反馈

- 文档问题：检查 `docs/` 文件夹
- 代码问题：参考 `IMPLEMENTATION_GUIDE.md` 中的 Q&A 部分
- 性能优化：后续 Phase 5 讨论

---

**版本**: 1.0 | **更新时间**: 2024-12-14 | **状态**: Phase 2（代码实现）准备中
