use anyhow::{Context, Result};
use wasmtime::{
    Config, Engine, Store,
    component::{Component, Linker, ResourceTable, Val},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView};

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for State {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

fn main() -> Result<()> {
    // 1. 配置 wasmtime engine
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(false);

    let engine = Engine::new(&config)?;

    // 2. 创建 Store 和 WASI 上下文
    let mut store = Store::new(
        &engine,
        State {
            wasi: WasiCtxBuilder::new()
                .inherit_stdio()
                .inherit_network() // 启用网络支持
                .build(),
            table: ResourceTable::default(),
        },
    );

    // 3. 创建 Linker 并添加 WASI 支持
    let mut linker: Linker<State> = Linker::new(&engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker)?;

    // 4. 加载 scheduler 组件
    let component = Component::from_file(
        &engine,
        "plugins/scheduler/target/wasm32-wasip2/debug/scheduler.wasm",
    )
    .context("failed to load scheduler component")?;

    // 5. 实例化组件
    let instance = linker
        .instantiate(&mut store, &component)
        .context("failed to instantiate component")?;

    // 6. 获取 run-scenario 函数
    let run_scenario = instance
        .get_typed_func::<(String,), (Result<String, String>,)>(&mut store, "run-scenario")
        .context("failed to find run-scenario function")?;

    // 7. 准备测试场景 YAML
    let scenario_yaml = r#"
scenario:
  id: test-001
  description: Simple test
  load:
    total_users: 2
    ramp_duration: 1s
    test_duration: 5s
    ip_pool:
      - 127.0.0.1
  workflow:
    - id: step1
      type: http
      name: Test Request
      params:
        method: GET
        url: http://example.com/
      think_time: 1s
"#;

    // 8. 调用函数
    println!("🚀 Running load test scenario...\n");
    let (result,) = run_scenario.call(&mut store, (scenario_yaml.to_string(),))?;

    // 9. 处理结果
    match result {
        Ok(summary) => {
            println!("✅ Test completed successfully!");
            println!("\n📊 Results:\n{}", summary);
        }
        Err(error) => {
            eprintln!("❌ Test failed: {}", error);
        }
    }

    run_scenario.post_return(&mut store)?;

    Ok(())
}
