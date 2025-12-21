use anyhow::{Context, Result};
use ntx::wasm_engine::{ComponentEngine, EngineConfig};
use std::env;

fn main() -> Result<()> {
    let component_path =
        env::var("NTX_COMPONENT").context("set NTX_COMPONENT=<path-to-your-wasm-component>")?;

    let entry = env::var("NTX_COMPONENT_ENTRY").unwrap_or_else(|_| "handle-packet".into());
    let input = env::var("NTX_COMPONENT_INPUT").unwrap_or_else(|_| "hello".into());

    let cfg = EngineConfig {
        component_path: component_path.into(),
        entry_candidates: vec![entry],
    };

    let mut engine = ComponentEngine::new(cfg)?;
    let res = engine.call_demo(&input)?;

    println!("did_work={} tx_packets={}", res.did_work, res.tx.len());
    Ok(())
}
