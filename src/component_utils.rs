use anyhow::{Result, bail};
use wasmtime::Store;
use wasmtime::component::{ComponentExportIndex, Func, Instance, types::ComponentItem};

use crate::State;

// Locate interface export parent (component instance)
pub fn find_iface_parent(
    store: &mut Store<State>,
    inst: &Instance,
    candidates: &[&str],
) -> Result<ComponentExportIndex> {
    for name in candidates {
        if let Some((item, idx)) = inst.get_export(&mut *store, None, name) {
            if matches!(item, ComponentItem::ComponentInstance(_)) {
                return Ok(idx);
            } else {
                println!("找到非接口导出：{item:#?}");
            }
        }
    }
    bail!(
        "找不到接口导出：候选 = {candidates:?}\n请用 `wasm-tools component wit demo.wasm` 查看实际导出名/版本，并在 WAC 顶层正确 `export`。"
    );
}

// Top-level func lookup
pub fn find_top_level_func(
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

// Get func from interface namespace
pub fn get_func_from_iface(
    store: &mut Store<State>,
    inst: &Instance,
    parent: &ComponentExportIndex,
    func_name: &str,
) -> Option<Func> {
    let (_item, func_idx) = inst.get_export(&mut *store, Some(parent), func_name)?;
    inst.get_func(&mut *store, func_idx)
}
