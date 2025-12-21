use anyhow::{Context, Result};
use ntx::wasm_engine::{ComponentEngine, EngineConfig};
use std::env;

fn main() -> Result<()> {
    let component_path =
        env::var("NTX_COMPONENT").context("set NTX_COMPONENT=<path-to-your-wasm-component>")?;

    let cfg = EngineConfig {
        component_path: component_path.into(),
        // This example calls the typed `run()` export, so entry candidates are irrelevant.
        entry_candidates: vec![],
    };

    let mut engine = ComponentEngine::new(cfg)?;
    engine.run()?;

    println!("guest run() completed");
    Ok(())
}
