# 业务组件（HTTP / FTP）Wasm Component 框架方案

## 目标
- 将 HTTP、FTP 等协议处理逻辑封装为独立的 wasm32-wasip2 Component。
- 通过统一的 WIT 契约（参见 `plugins/runner/wit/progress.wit`、`wit/http-component.wit`、`wit/ftp-component.wit`）与 Runner Core 对接。
- 提供标准化的构建、装载、测试和灰度策略，方便在多环境复用。

## 架构要点
1. **契约即 API**：所有跨组件交互只通过 WIT 接口完成，禁止共享内存或宿主特权调用。
2. **能力最小化**：组件运行时仅暴露受限的 WASI 能力（网络句柄、时钟、随机数等）；其余由宿主代理。
3. **可观测性**：每次状态变更与请求处理都转化为 `ProgressUpdate`，再写入 `progress.wit` 的 `events` 接口。
4. **灰度控制**：通过 `feature_flags.toml` 控制 `runner.http_component`、`runner.ftp_component` 等开关，并支持 CLI 覆盖。

## 构建 / 绑定生成
在每个组件目录（示例：`plugins/http-component/`）执行：

```bash
# 1. 构建 wasm32-wasip2 产物
cargo build --release --target wasm32-wasip2 -p http-component

# 2. 为宿主生成 Rust 绑定（以 HTTP 为例）
wit-bindgen rust --world http-component \
  -o plugins/http-component/src/bindings \
  wit/http-component.wit

# 2b. 生成 FTP 组件绑定
wit-bindgen rust --world ftp-component \
  -o plugins/ftp-component/src/bindings \
  wit/ftp-component.wit

# 3. （可选）为 Runner Core 生成进度接口绑定
wit-bindgen rust --world progress-runner \
  -o plugins/runner/src/progress_bindings \
  plugins/runner/wit/progress.wit
```

## 运行与装载
宿主（Runner Core）使用 `wasmtime` 或自定义运行时装载：

```bash
wasmtime run \
  --env FEATURE_runner_http_component=true \
  --dir config=./res/config \
  target/wasm32-wasip2/release/http_component.wasm
```

加载流程：
1. 读取组件 manifest（包含版本、所需接口）。
2. 校验 WIT SHA（与仓库中的 `wit/*.wit` 一致）。
3. 注入受限的 WASI 能力与 Runner 自定义导入（如日志、进度事件）。
4. 调用 `server.start_listen` / `control.start_server` 等入口。

## 测试与 CI
- **单元测试**：`cargo test -p http-component`（普通 Rust 单元）。
- **组件测试**：编译后使用 `wasmtime` 加载 `.wasm`，运行 smoke cases。
- **契约验证**：`wac validate plugins/runner/wit/progress.wit wit/http-component.wit wit/ftp-component.wit`。
- **集成测试**：在 Runner 场景脚本中启用对应特性开关，确保 ProgressUpdate 流程贯通。
- **基线采集**：复用 `docs/plan/baseline.md` 的 jemalloc/Prometheus 步骤，对比不同组件实装的资源占用。

CI 建议步骤：
1. `cargo fmt && cargo clippy`（全仓）。
2. `cargo test --workspace`。
3. `cargo build --release --target wasm32-wasip2 -p http-component -p ftp-component`。
4. `wac validate` 校验 WIT。
5. `scripts/run_component_smoke.sh http`、`scripts/run_component_smoke.sh ftp`（调用 wasmtime）。
6. 上传 `.wasm` + 版本元数据到内部制品库。

## 部署与灰度
- 默认在 `feature_flags.toml` 中关闭 HTTP/FTP component，先在 staging 手动开启。
- Runner Core 读取 `feature_flags` 并决定是否加载 wasm component 或回退至老实现。
- 灰度指标：请求成功率、延迟、内存占用、进度事件延迟。若任一超过阈值自动回退。

## 文档索引
- 进度接口：`plugins/runner/wit/progress.wit`
- HTTP Component 契约：`wit/http-component.wit`
- FTP Component 契约：`wit/ftp-component.wit`
- 稀疏数据模型：`plugins/runner/docs/plan/sparse_model.md`
- 特性开关策略：`plugins/runner/docs/plan/feature_flags.md`
- 基线采集：`plugins/runner/docs/plan/baseline.md`

## 后续动作
1. 在 `plugins/` 下创建 `http-component`、`ftp-component` crate，并接入上述 WIT。
2. 更新 Runner Core，利用 `wasmtime::component::Component` 装载 `.wasm` 并桥接到 `progress.wit`。
3. 将 `wac validate`、`wasmtime smoke` 步骤纳入 CI。
4. 按 `feature_flags` 策略开展灰度与基线采集。
