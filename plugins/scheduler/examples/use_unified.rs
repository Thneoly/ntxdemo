use anyhow::Result;
use wasmtime::component::*;
use wasmtime::{Config, Engine, Store};

// This example demonstrates using the unified scheduler component
// Currently wraps scheduler-core with parse and type functionality

fn main() -> Result<()> {
    println!("🚀 Unified Scheduler Component Demo\n");

    // Configure wasmtime with component model support
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;

    let mut store = Store::new(&engine, ());

    // Load the unified component
    println!("📦 Loading unified component...");
    let component_path = "composed/target/unified_scheduler.wasm";
    let component = Component::from_file(&engine, component_path)?;
    println!("✅ Component loaded: {}\n", component_path);

    // Create a linker with WASI support
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker)?;

    // Create WASI context
    let wasi = wasmtime_wasi::WasiCtxBuilder::new()
        .inherit_stdio()
        .inherit_env()
        .build();
    store.data_mut().wasi = wasi;

    println!("🔗 Instantiating component...");
    let instance = linker.instantiate(&mut store, &component)?;
    println!("✅ Component instantiated\n");

    // Example: Parse a YAML scenario
    println!("📝 Testing parse-scenario function...");
    let yaml_input = r#"
version: "1.0"
name: "test-workflow"
description: "A simple test workflow"
workflows:
  nodes:
    - id: "step1"
      type: "action"
      name: "First Step"
"#;

    println!("Input YAML:\n{}\n", yaml_input);

    // Get the parse-scenario function
    // Note: Actual signature depends on generated bindings
    println!("🔍 Looking up parse-scenario export...");

    // List all exports
    println!("Available exports:");
    for (name, _) in instance.exports(&mut store) {
        println!("  - {}", name);
    }

    println!("\n✅ Demo complete!");
    println!("\n📚 Next steps:");
    println!("  1. Implement Guest traits in executor component");
    println!("  2. Complete actions-http component bindings");
    println!("  3. Re-run ./create_unified.sh for full composition");
    println!("  4. Update this demo with actual function calls");

    Ok(())
}

// Add to Cargo.toml:
// [dependencies]
// wasmtime = { version = "26", features = ["component-model"] }
// wasmtime-wasi = "26"
// anyhow = "1.0"
