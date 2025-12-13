use std::{env, fs};

use anyhow::{Context, Result, bail};

mod network;
use wasmtime::{
    Config, Engine, Store,
    component::{
        Component, ComponentExportIndex, Func, Instance, Linker, ResourceTable,
        types::ComponentItem,
    },
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::add_to_linker_sync};
struct State {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for State {
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
    config.async_support(false);

    let engine = Engine::new(&config)?;
    let mut store = Store::new(
        &engine,
        State {
            wasi: WasiCtxBuilder::new()
                .inherit_stdio()
                .inherit_network()
                .build(),
            table: ResourceTable::default(),
        },
    );
    let mut linker: Linker<State> = Linker::new(&engine);
    add_to_linker_sync(&mut linker)?;
    let component_path = env::var("SCHEDULER_COMPONENT")
        .unwrap_or_else(|_| "plugins/scheduler/wac/scheduler-composed.wasm".into());
    let component_path_display = component_path.clone();
    let component = Component::from_file(&engine, component_path)
        .with_context(|| format!("载入组件失败: {component_path_display}"))?;

    let instance = linker
        .instantiate(&mut store, &component)
        .context("failed to instantiate component")?;

    let func = find_top_level_func(&mut store, &instance, &["run-scenario"])?;
    let typed = func
        .typed::<(&str,), (Result<String, String>,)>(&store)
        .context("run-scenario 签名检查失败")?;

    println!(
        "开始执行 run-scenario，输入 YAML 长度 {} 字节",
        scenario.len()
    );
    match typed.call(&mut store, (&scenario,))?.0 {
        Ok(summary) => {
            println!("✅ 执行成功: {summary}");
        }
        Err(err) => {
            println!("❌ 执行失败: {err}");
        }
    }
    Ok(())
}

// 顶层找接口导出的"父索引"，用于进入接口命名空间
#[allow(unused)]
fn find_iface_parent(
    store: &mut Store<State>,
    inst: &Instance,
    candidates: &[&str],
) -> Result<ComponentExportIndex> {
    for name in candidates {
        if let Some((item, idx)) = inst.get_export(&mut *store, None, name) {
            if matches!(item, ComponentItem::ComponentInstance(_)) {
                return Ok(idx);
            } else {
                println!("找到非接口导出：{:#?}", item);
            }
        } else {
            println!("未找到候选接口导出：{name}");
        }
    }
    bail!(
        "找不到接口导出：候选 = {candidates:?}\n请用 `wasm-tools component wit demo.wasm` 查看实际导出名/版本，并在 WAC 顶层正确 `export`。"
    );
}

// 顶层函数查找：在顶层导出中按候选名查找 func
#[allow(unused)]
fn find_top_level_func(
    store: &mut Store<State>,
    inst: &Instance,
    candidates: &[&str],
) -> Result<Func> {
    for name in candidates {
        if let Some((item, idx)) = inst.get_export(&mut *store, None, name) {
            if matches!(item, ComponentItem::ComponentFunc(_)) {
                if let Some(f) = inst.get_func(&mut *store, idx) {
                    return Ok(f);
                }
            }
        }
    }
    bail!(
        "找不到顶层函数导出：候选 = {candidates:?}。请用 `wasm-tools component wit <你的 wasm>` 确认实际导出名。"
    );
}

// 从接口命名空间获取函数
#[allow(unused)]
fn get_func_from_iface(
    store: &mut Store<State>,
    inst: &Instance,
    parent: &ComponentExportIndex,
    func_name: &str,
) -> Option<Func> {
    let (_item, func_idx) = inst.get_export(&mut *store, Some(parent), func_name)?;
    inst.get_func(&mut *store, func_idx)
}
