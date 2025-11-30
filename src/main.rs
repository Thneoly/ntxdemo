use std::{env, fs};

use anyhow::{Context, Result};
use wasmtime::{Config, Engine, Store, component::ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::add_to_linker_sync};

wasmtime::component::bindgen!({
    path: "plugins/scheduler/scheduler/wit/world.wit",
    world: "scheduler-component",
});

struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn main() -> Result<()> {
    let default_scenario = "plugins/scheduler/res/simple_scenario.yaml";
    let scenario_path = env::args()
        .nth(1)
        .unwrap_or_else(|| default_scenario.to_string());
    let scenario = fs::read_to_string(&scenario_path)
        .with_context(|| format!("读取场景文件失败: {scenario_path}"))?;

    let mut config = Config::new();
    config.wasm_component_model(true);

    let engine = Engine::new(&config)?;
    let mut linker = wasmtime::component::Linker::new(&engine);
    add_to_linker_sync(&mut linker)?;

    let host_state = HostState {
        wasi: WasiCtxBuilder::new()
            .inherit_stdio()
            .inherit_network()
            .build(),
        table: ResourceTable::default(),
    };
    let mut store = Store::new(&engine, host_state);

    let component_path = env::var("SCHEDULER_COMPONENT").unwrap_or_else(|_| {
        "plugins/scheduler/target/wasm32-wasip2/debug/scheduler_composed.wasm".into()
    });
    let component_path_display = component_path.clone();
    let component = wasmtime::component::Component::from_file(&engine, component_path)
        .with_context(|| format!("载入组件失败: {component_path_display}"))?;

    let scheduler = SchedulerComponent::instantiate(&mut store, &component, &linker)
        .context("实例化 scheduler 组件失败")?;

    println!(
        "开始执行 run-scenario，输入 YAML 长度 {} 字节",
        scenario.len()
    );
    match scheduler.call_run_scenario(&mut store, &scenario)? {
        Ok(summary) => {
            println!("✅ 执行成功: {summary}");
        }
        Err(err) => {
            println!("❌ 执行失败: {err}");
        }
    }

    Ok(())
}
