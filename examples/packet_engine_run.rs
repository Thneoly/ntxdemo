use anyhow::Result;
use ntx::wasm_engine::{ComponentEngine, EngineConfig};
use std::env;

fn main() -> Result<()> {
    let component_path = env::var("NTX_COMPONENT")
        .unwrap_or_else(|_| "./component/wac/scheduler-composed.wasm".to_string());

    let cfg = EngineConfig {
        component_path: component_path.into(),
    };

    let mut engine = ComponentEngine::new(cfg)?;
    let n = engine.notify_rx(Vec::new(), Vec::new())?;
    println!("notify-rx completed (n={n})");
    Ok(())
}
