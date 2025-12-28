use anyhow::Result;
use ntx::wasm_engine::{ComponentEngine, EngineConfig};
use std::env;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let component_path = env::var("NTX_COMPONENT")
        .unwrap_or_else(|_| "./component/wac/scheduler-composed.wasm".to_string());

    let cfg = EngineConfig {
        component_path: component_path.into(),
    };

    let engine = ComponentEngine::new(cfg).await?;
    // End-state: RX is enqueued into the host rx-ring; the guest pulls via import.
    engine.rx_ring().enqueue_batch(Vec::new(), Vec::new());
    println!("enqueued empty rx batch");
    Ok(())
}
