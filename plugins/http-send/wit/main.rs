use anyhow::{Context, Ok, Result, bail};
use wasmtime::{
    Engine, Store,
    component::{Component, ComponentExportIndex, Func, HasSelf, Instance, Linker, ResourceTable, Val,types::ComponentItem}
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::{add_to_linker_sync}};

wasmtime::component::bindgen!({
    path: "wit/host",
    world: "host",
});

use crate::host::core::hostitf::Host;

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl Host for State {
    fn log(&mut self, msg: String) {
        println!("Guest says: {}", msg);
    }
    fn connect(&mut self, addr: String) -> String {
        println!("Connecting to {}", addr);
        format!("Connection to {}", addr)
    }
    fn read(&mut self) -> String {
        // Placeholder implementation
        format!("Data from read")
    }
    fn write(&mut self, data: String) -> u32{
        println!("Writing data: {}", data);
        0
    } 
    fn close(&mut self) -> bool {
        println!("Closing connection");
        true
    }
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
    let engine = Engine::default();
    let mut store = Store::new(
        &engine,
        State {
            wasi: WasiCtxBuilder::new().inherit_stdio().build(),
            table: ResourceTable::default(),
        },
    );
    let mut linker: Linker<State> = Linker::new(&engine);
    add_to_linker_sync(&mut linker)?;
    Host_::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s)?;
    let component = Component::from_file(&engine, "plugins/tcp/target/wasm32-wasip2/debug/tcp.wasm")
        .context("failed to create component from file")?;

    let instance = linker
        .instantiate(&mut store, &component)
        .context("failed to instantiate component")?;
    println!("Instance:\n{:#?}", instance);
    let tcpitf = find_iface_parent(&mut store, &instance, &["customer:tcp/tcpitf@0.1.0"])?;
    println!("找到接口命名空间：{tcpitf:?}");
    let connect = get_func_from_iface(&mut store, &instance, &tcpitf, "connect")
        .context("failed to find `connect` function in interface")?;
    println!("找到 connect 函数：{connect:?}");
    let mut result = [Val::String("".to_string())];
    connect.call(&mut store, &[], &mut result).context("failed to call `connect`")?;
    let _ = connect.post_return(&mut store);
    let params = [Val::U32(10086)];
    let start = get_func_from_iface(&mut store, &instance, &tcpitf, "start")
        .context("failed to find `start` function in interface")?;
    let mut result_start = [Val::Bool(false)];
    start.call(&mut store, &params, &mut result_start).context("failed to call `start`")?;

    Ok(())
}

// 顶层找接口导出的“父索引”，用于进入接口命名空间
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
    bail!("找不到顶层函数导出：候选 = {candidates:?}。请用 `wasm-tools component wit <你的 wasm>` 确认实际导出名。");
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