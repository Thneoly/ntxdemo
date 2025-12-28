/// Regression test: verifies host wiring for `ntx:hostnet/*` imports.
///
/// This intentionally does **not** invoke `cargo component build` (which is not
/// guaranteed to be available/working in all CI environments for this repo).
/// Instead, it uses an already-built component artifact if present.
#[test]
fn hostnet_imports_are_wired() {
    // Use the same component location that other integration tests expect.
    // If the output path changes in the future, reuse the existing helper.
    // Keep consistent with `tests/wasm_packet_guest_integration.rs`.
    let component_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("wasm32-wasip2")
        .join("debug")
        .join("packet_guest.wasm");

    if !component_path.exists() {
        eprintln!(
            "skipping hostnet linker smoke test: component artifact not found at {}",
            component_path.display()
        );
        return;
    }

    // Instantiate the engine and call the smoke export.
    // NOTE: This uses the host's generated bindings (not the guest's).
    let cfg = ntx::wasm_engine::EngineConfig { component_path };

    let mut engine =
        ntx::wasm_engine::ComponentEngine::new(cfg).expect("failed to create ComponentEngine");

    // Instantiation already exercises import resolution.
    // If host wiring is missing, `ComponentEngine::new` would fail above.
    engine.enqueue_rx_batch(Vec::new(), Vec::new());
}
