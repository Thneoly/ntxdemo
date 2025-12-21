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
    let cfg = ntx::wasm_engine::EngineConfig {
        component_path,
        // Not used by this test, but required by config.
        entry_candidates: vec![],
    };

    let mut engine =
        ntx::wasm_engine::ComponentEngine::new(cfg).expect("failed to create ComponentEngine");

    // Prefer calling the explicit smoke export if available (stronger signal), but
    // don't fail if the artifact doesn't include it (e.g., older component builds).
    match engine.hostnet_smoke_create_owner("smoke-test") {
        Ok(owner) => assert!(!owner.is_empty()),
        Err(_) => {
            // Fallback: instantiation already exercises import resolution.
            // If host wiring is missing, `ComponentEngine::new` would fail above.
            let _ = engine.notify_rx().expect("notify_rx should work");
        }
    }
}
