# Ntx (演示)

这是一个演示仓库，包含若干插件（位于 `plugins/`）和一个顶层的 Rust 可执行/库。仓库中的 build 脚本会监视插件目录的变动，并在必要时为指定插件触发子构建或运行脚本（例如将插件构建为 wasm 目标）。

## 主要特性
- 自动检测 `plugins/core`、`plugins/demo`、`plugins/tcp-client`、`plugins/wac` 的文件变更。
- 对 `core`、`demo`、`tcp-client` 在变更时执行 `cargo build --target wasm32-wasip2`。
- 对 `wac` 在变更时执行 `run.sh`（通过 `sh run.sh` 运行）。
- 状态文件写入到 Cargo 的 `OUT_DIR`（或回退到 `target/`），避免将构建状态提交到版本控制。
- 在 CI 或快速本地构建时可以通过环境变量关闭插件自动构建（参见下文）。

## 环境准备
安装一些常用工具：

```bash
cargo install cargo-binstall
cargo binstall cargo-component wit-bindgen-cli wasmtime-cli wasm-tools wit-deps-cli cargo-expand -y
```

（按需安装上面工具中的子集。）

## 构建与运行

在仓库根目录运行：

```bash
cargo build
```

或者运行：

```bash
# 需要启动 TCP 服务并监听 8080 端口, 可以使用 nc 工具
cargo run
```

这些命令会触发顶层构建并执行 `build.rs`。当 `plugins/*` 下有变更时，`build.rs` 可能会在对应子目录执行 `cargo build --target wasm32-wasip2` 或 `sh run.sh`。

## 控制插件构建（跳过/启用）

如果你想在 CI 或快速本地开发时跳过插件自动构建（仅构建顶层），可以设置环境变量 `DISABLE_PLUGIN_BUILDS`：

```bash
# 跳过插件自动构建（值为 1 或 true 将被视为启用）
DISABLE_PLUGIN_BUILDS=1 cargo build
```

当 `DISABLE_PLUGIN_BUILDS` 被设置时，build 脚本仍会计算并记录插件目录的哈希（状态会更新），但不会执行 `cargo build` 或 `run.sh`。

如果你更希望在跳过时不更新状态（以便重新启用时仍然触发一次完整构建），请告诉我，我可以修改行为。

## 状态文件位置

插件变更的状态记录（哈希）会写到 Cargo 提供的 `OUT_DIR` 目录下的 `plugin_build_state` 文件。如果 `OUT_DIR` 不可用，脚本会回退到仓库的 `target/` 目录。该文件默认不会被添加到 VCS，但如果你希望将其放在别处（或加入 `.gitignore`），可以调整脚本配置。

## 插件目录与忽略规则

为避免扫描大量构建产物，脚本会忽略以下目录：

- `*.git`、`node_modules`（任意位置），
- 严格忽略 `target` 目录（尤其是 `plugins/*/target` 下的子目录）。

如果你的项目结构包含其他需要忽略的目录，请告知，我会加入到忽略列表中。

## 故障排查

- 如果子插件构建失败，当前实现会让顶层构建失败（panic），并在终端输出子进程的退出状态与错误信息。
- 若要将子构建失败改为仅打印警告并继续主构建，请说明，我可以把错误处理改为非致命日志。
- 若发现 build 脚本没有触发预期子构建，确认：
	- 你是否修改了 `plugins/<name>` 下的文件（注意 `target` 被忽略）；
	- 是否设置了 `DISABLE_PLUGIN_BUILDS`；
	- 检查 `OUT_DIR/plugin_build_state` 中保存的哈希以判断上次构建状态。

## 示例 — 快速开发流程

1. 修改插件 `plugins/core/src/lib.rs`。
2. 在仓库根运行（默认会触发插件子构建）：

```bash
cargo build
```

3. 若要快速跳过插件构建：

```bash
DISABLE_PLUGIN_BUILDS=1 cargo build
```

## 其他

可以使用 `nc -l 127.0.0.1 8080` 监听端口 开启TCP 监听端口。