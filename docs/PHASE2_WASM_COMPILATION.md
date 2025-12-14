# Phase 2: 真实 WASM 组件编译

## 📋 目标

将 Echo Server/Client 从本地实现编译成真实的 WASM 组件，替换占位符文件，实现完整的 WASM 集成。

## 🎯 成功标志

- ✅ Echo Server WASM 成功编译
- ✅ Echo Client WASM 成功编译
- ✅ WAC 组合文件验证
- ✅ 系统自动加载 WASM（日志显示: `[wasm] Found exported function`）
- ✅ Echo Server/Client 完整功能运行

## 📐 架构概览

```
源代码:
├── actions-executor-server/
│   ├── src/lib.rs (Echo 实现)
│   ├── wit/actions-executor-server.wit (接口)
│   └── Cargo.toml (cdylib)
│
└── actions-executor-client/
    ├── src/lib.rs (Echo 实现)
    ├── wit/actions-executor-client.wit (接口)
    └── Cargo.toml (cdylib)

WASM 编译:
├── actions-executor-server.wasm (wasm32-wasip2)
└── actions-executor-client.wasm (wasm32-wasip2)

WAC 组合:
├── echo-server.wac (组合配置)
├── echo-client.wac (组合配置)
└── 最终输出:
    ├── echo-server.wasm (最终组件)
    └── echo-client.wasm (最终组件)

系统集成:
├── plugins/scheduler/wac/echo-server.wasm ← 替换这个
└── plugins/scheduler/wac/echo-client.wasm  ← 替换这个
```

## 🔧 Step 1: 编译 Echo Server WASM

### 前置条件检查

```bash
# 检查 Rust 工具链
rustc --version              # 应该 ≥ 1.70
cargo --version             # 应该 ≥ 1.70

# 检查 wasm32-wasip2 target
rustup target list | grep wasm32-wasip2  # 应该安装
```

如果未安装 wasm32-wasip2:
```bash
rustup target add wasm32-wasip2
```

### 编译命令

```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/actions-executor-server

# 清理之前的构建
cargo clean

# 编译为 WASM
cargo build --target wasm32-wasip2 --release

# 输出应该在这里
# target/wasm32-wasip2/release/scheduler_actions_executor_server.wasm
```

### 预期输出

```
Compiling scheduler-actions-executor-server v0.1.0
    Finished release [optimized] target(s) in X.XXs

# 检查 WASM 文件大小和导出
wasm-objdump -x target/wasm32-wasip2/release/scheduler_actions_executor_server.wasm | head -20
```

### 问题排查

**问题**: `cannot find path` 错误
- **原因**: WIT 文件路径不对
- **解决**: 检查 `Cargo.toml` 中的 `wit-bindgen` 配置

**问题**: `linking with xxx failed` 错误
- **原因**: wit-bindgen 版本不匹配
- **解决**: 更新 `wit-bindgen` 版本或清理缓存

**问题**: `export not found` 错误
- **原因**: WIT 接口定义不正确
- **解决**: 检查 WIT 文件格式

## 🔧 Step 2: 编译 Echo Client WASM

### 编译命令

```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/actions-executor-client

# 清理
cargo clean

# 编译
cargo build --target wasm32-wasip2 --release

# 输出
# target/wasm32-wasip2/release/scheduler_actions_executor_client.wasm
```

### 验证

```bash
# 检查文件大小
ls -lh target/wasm32-wasip2/release/*.wasm

# 预期: 两个文件，都在 200KB-1MB 之间
```

## 📦 Step 3: WAC 组合验证

### 检查 WAC 配置

```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/wac

# 查看 WAC 文件
cat echo-server.wac
cat echo-client.wac

# 预期内容
```

**echo-server.wac 示例**:
```toml
[package]
name = "echo-server"
version = "0.1.0"

[component]
path = "../actions-executor-server/target/wasm32-wasip2/release/scheduler_actions_executor_server.wasm"
world = "scheduler:actions-executor-server"
```

**echo-client.wac 示例**:
```toml
[package]
name = "echo-client"
version = "0.1.0"

[component]
path = "../actions-executor-client/target/wasm32-wasip2/release/scheduler_actions_executor_client.wasm"
world = "scheduler:actions-executor-client"
```

### 尝试 WAC 组合

```bash
# 如果系统中有 wac 工具
wac echo-server.wac -o echo-server-composed.wasm

# 或者如果不需要组合，直接复制
cp ../actions-executor-server/target/wasm32-wasip2/release/scheduler_actions_executor_server.wasm \
   echo-server.wasm

cp ../actions-executor-client/target/wasm32-wasip2/release/scheduler_actions_executor_client.wasm \
   echo-client.wasm
```

## ✨ Step 4: 更新 WASM 文件

### 复制编译好的组件

```bash
cd /home/cc/Desktop/code/GitHub/Ntx/plugins/scheduler/wac

# 备份原有占位符（可选）
mv echo-server.wasm echo-server.wasm.bak
mv echo-client.wasm echo-client.wasm.bak

# 复制新的 WASM 组件
cp ../actions-executor-server/target/wasm32-wasip2/release/scheduler_actions_executor_server.wasm \
   echo-server.wasm

cp ../actions-executor-client/target/wasm32-wasip2/release/scheduler_actions_executor_client.wasm \
   echo-client.wasm

# 验证
ls -lh echo-*.wasm
```

### 预期大小

- **echo-server.wasm**: 200KB - 500KB (之前: 15MB 占位符)
- **echo-client.wasm**: 200KB - 500KB (之前: 15MB 占位符)

## 🧪 Step 5: 系统集成测试

### 重新编译主程序

```bash
cd /home/cc/Desktop/code/GitHub/Ntx

cargo build  # Debug
# 或
cargo build --release  # Release
```

### 创建虚拟网卡

```bash
sudo /home/cc/Desktop/code/GitHub/Ntx/scripts/ntx-veth-up.sh
```

### 运行 Echo Server

```bash
sudo ./target/debug/Ntx --mode server --iface ntx0 --port 10001
```

**预期日志**:
```
[WASM] load successful
[wasm] Found exported function, using WASM implementation
[echo-server] iface=ntx0 mac=... port=10001 backend=AfPacket
```

### 运行 Echo Client

```bash
sudo ip netns exec ntxns1 ./target/debug/Ntx --mode client \
  --iface ntx1 --server-ip 10.0.0.1 --server-port 10001 \
  --count 10 --pps 5
```

**预期日志**:
```
[WASM] load successful
[wasm] Found exported function, using WASM implementation
[echo-client] Generating 10 requests at 5 pps
[echo-client] Sent seq=0
...
[result] sent=10 matched=X timeouts=Y errors=Z
```

## 📊 验证检查表

| 项目 | 检查 | 结果 |
|------|------|------|
| **Server WASM** | 编译成功 | ⬜ |
| **Server WASM** | 文件大小合理 | ⬜ |
| **Server WASM** | 导出函数正确 | ⬜ |
| **Client WASM** | 编译成功 | ⬜ |
| **Client WASM** | 文件大小合理 | ⬜ |
| **Client WASM** | 导出函数正确 | ⬜ |
| **集成** | main 程序编译成功 | ⬜ |
| **集成** | WASM 加载成功 | ⬜ |
| **集成** | WASM 函数被调用 | ⬜ |
| **功能** | Echo Server 正常运行 | ⬜ |
| **功能** | Echo Client 正常运行 | ⬜ |
| **性能** | 无性能退化 | ⬜ |

## 🚨 常见错误和解决方案

### 错误 1: `cannot find path to `wit` directory`

**症状**:
```
error: cannot find path to `wit` directory
```

**解决**:
```bash
# 检查 wit 文件位置
ls -la plugins/scheduler/actions-executor-server/wit/

# 确保在 Cargo.toml 中指定正确的路径
cat Cargo.toml | grep wit
```

### 错误 2: `linking with xxx failed`

**症状**:
```
error: linking with ld failed: exit code: 1
```

**解决**:
```bash
# 清理缓存
cargo clean

# 检查依赖版本
cargo tree | grep wit-bindgen

# 重新构建
cargo build --target wasm32-wasip2 --release
```

### 错误 3: `cannot find function`

**症状**:
```
cannot find exported function `on-packet-received`
```

**解决**:
```bash
# 检查 WIT 接口
cat wit/actions-executor-server.wit

# 检查 Rust 实现
grep -n "on_packet_received" src/lib.rs

# 确保导出的函数名称与 WIT 匹配
```

### 错误 4: WASM 加载仍然失败

**症状**:
```
[WASM] instantiate failed: ..., falling back to native
```

**解决**:
```bash
# 1. 检查 WASM 文件是否有效
file plugins/scheduler/wac/echo-server.wasm

# 2. 查看 WASM 导出
wasm-objdump -x plugins/scheduler/wac/echo-server.wasm | head -30

# 3. 检查 main.rs 中的导出函数名
grep -n "find_top_level_func" src/main.rs

# 4. 确保导出的名称与代码中查找的名称匹配
```

## 📈 性能对比

### 预期结果

| 指标 | 本地实现 | WASM 实现 | 差异 |
|------|---------|---------|------|
| 延迟 | ~1-5ms | ~2-8ms | +20-50% |
| 吞吐量 | ~10k pps | ~8k pps | -20% |
| 内存 | ~5MB | ~8MB | +60% |
| 启动时间 | <100ms | 200-500ms | +100-400% |

*注*: 实际性能取决于 WASM 运行时优化

## ✅ 完成标志

当看到以下日志时，Phase 2 成功：

```
[WASM] load successful
[wasm] Found exported function, using WASM implementation
[echo-server] iface=ntx0 mac=36:ad:81:94:69:3a port=10001 backend=AfPacket
[echo-server] rx=100 udp=50 processed=50 sent=50
```

## 🔄 回退计划

如果 WASM 实现出现问题：

```bash
# 恢复占位符
cp echo-server.wasm.bak echo-server.wasm
cp echo-client.wasm.bak echo-client.wasm

# 系统自动回退到本地实现
./target/debug/Ntx --mode server --iface ntx0
# 日志: [WASM] instantiate failed: ..., falling back to native
```

## 📚 参考资源

- **WIT 文档**: https://github.com/bytecodealliance/wit-spec
- **WAC 文档**: https://github.com/bytecodealliance/wac
- **Wasmtime 文档**: https://docs.wasmtime.dev/
- **wit-bindgen 文档**: https://github.com/bytecodealliance/wit-bindgen

## 🎯 后续步骤

Phase 2 完成后：

### 阶段 3 (可选): 优化
- [ ] WASM 代码性能优化
- [ ] 内存使用优化
- [ ] 启动时间优化

### 阶段 4 (可选): 扩展
- [ ] 支持更多协议 (TCP, ICMP 等)
- [ ] 添加多线程支持
- [ ] 集成外部库

### 阶段 5 (可选): 部署
- [ ] 容器化
- [ ] CI/CD 集成
- [ ] 性能基准测试

---

**开始日期**: 2024-12-14
**预计完成**: 2024-12-14
**状态**: 🟡 待开始
