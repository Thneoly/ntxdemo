/// Integration test for the WAC-composed scheduler component.
///
/// End-state contract (see `component/doc/HOST.md`):
/// - Host loads `./component/wac/scheduler-composed.wasm`.
/// - Host calls export `ntx:scenario-scheduler/scheduler-component@0.1.0#run` once.
/// - RX is NOT injected via any export (no `packet-ingest.notify-rx`).
/// - Guest `run()` pulls RX via host import `ntx:host/rx-ring@0.1.0`.
///
/// NOTE: This test will be enabled after the host-side rx-ring provider is implemented
///       and wired into the linker used by `ComponentEngine`.
#[test]
#[ignore = "end-state refactor: enable after rx-ring import is implemented in host linker"]
fn composed_scheduler_pulls_rx_via_rx_ring() {
    // Intentionally empty for now.
    // Once rx-ring is wired:
    // - Instantiate composed scheduler
    // - Start `run(config-dir)` on a dedicated thread
    // - Enqueue one rx batch into the host rx-ring provider
    // - Assert the batch is pulled/consumed (via metrics or a test-only event export)
}
