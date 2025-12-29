use anyhow::Result;

/// 回归检查：
/// 1) actions-executor 的 catalog API 必须可用
/// 2) 至少包含两个动作：udp-send-reply / udp-schedule-send
///
/// 说明：
/// - 这是一个 host 侧测试，会用 wasmtime 实例化 wasm32-wasip2 component。
/// - 运行此测试前需要先构建 component：
///   `cargo build -p actions-executor --target wasm32-wasip2`
/// - CI 如果要跑它，可以在 job 里先做上面的 build。
#[test]
fn actions_catalog_contains_expected_actions() -> Result<()> {
    // 注意：tests 的当前工作目录不保证是 workspace root。
    // 用 CARGO_MANIFEST_DIR 定位到本 crate，再回到 workspace root。
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        // .../component/tools/actions-catalog-gen
        .nth(3)
        .expect("failed to locate workspace root");

    let component_path = workspace_root.join("target/wasm32-wasip2/debug/actions_executor.wasm");

    if !component_path.exists() {
        let status = std::process::Command::new("cargo")
            .args([
                "build",
                "-p",
                "actions-executor",
                "--target",
                "wasm32-wasip2",
            ])
            .current_dir(workspace_root)
            .status()?;
        anyhow::ensure!(
            status.success(),
            "failed to build actions-executor wasm component"
        );
        anyhow::ensure!(
            component_path.exists(),
            "missing component at {} after build (cwd={})",
            component_path.display(),
            std::env::current_dir()?.display()
        );
    }

    let catalog = actions_catalog_gen::load_catalog_from_component_sync(&component_path)?;
    anyhow::ensure!(catalog.schema_version == 1, "schema_version must be 1");

    let ids: std::collections::BTreeSet<_> = catalog
        .actions
        .iter()
        .map(|a| a.summary.id.as_str())
        .collect();

    anyhow::ensure!(
        ids.contains("udp-send-reply"),
        "catalog missing udp-send-reply"
    );
    anyhow::ensure!(
        ids.contains("udp-schedule-send"),
        "catalog missing udp-schedule-send"
    );

    Ok(())
}
