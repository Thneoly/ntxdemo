use std::path::PathBuf;

use ntx::wasm_engine::shared_mem;
use ntx::wasm_engine::{ComponentEngine, EngineConfig};

fn packet_guest_component_path() -> PathBuf {
    // Build output convention for wasm32-wasip2 in this repo.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("wasm32-wasip2")
        .join("debug")
        .join("packet_guest.wasm")
}

#[test]
fn packet_guest_enqueue_notify_advances_tail() {
    // If the guest component wasn't built yet, build it as a component.
    let path = packet_guest_component_path();
    if !path.exists() {
        let status = std::process::Command::new("cargo")
            .args(["component", "build", "-p", "packet-engine", "-q"])
            .status()
            .expect("failed to invoke cargo component build");
        assert!(status.success(), "cargo component build failed");
    }

    assert!(
        path.exists(),
        "guest component not found at {}",
        path.display()
    );

    let cfg = EngineConfig {
        component_path: path,
        // Not used in this test but required by config.
        entry_candidates: vec!["notify-rx".into()],
    };

    let mut engine = ComponentEngine::new(cfg).expect("create component engine");

    // Before enqueue, control block should already be initialized when we enqueue.
    // (We validate after enqueue for determinism.)

    // Enqueue one packet.
    let payload = b"hello";
    engine.enqueue_rx(Some(7), payload).expect("enqueue_rx");

    // Validate control block after enqueue.
    let desc_mem_after_enqueue = engine.read_desc_buffer().expect("read desc buffer");
    let cb_after_enqueue =
        shared_mem::decode_control(&desc_mem_after_enqueue).expect("decode control after enqueue");
    assert_eq!(cb_after_enqueue.magic, shared_mem::NTX_MAGIC, "magic");
    assert_eq!(cb_after_enqueue.version, shared_mem::NTX_VERSION, "version");
    assert_eq!(
        cb_after_enqueue.desc_tail, 1,
        "tail should be 1 after enqueue"
    );
    assert_eq!(
        cb_after_enqueue.desc_head, 0,
        "head should still be 0 before consume"
    );

    // Call notify.
    let n = engine.notify_rx().expect("notify_rx");
    assert_eq!(n, 1, "guest should consume one descriptor");

    // Verify descriptor tail advanced in control block (tail == head == 1 after consume).
    let desc_mem = engine.read_desc_buffer().expect("read desc buffer");
    let cb = shared_mem::decode_control(&desc_mem).expect("decode control");
    assert_eq!(cb.desc_tail, 1, "tail should advance to 1");
    assert_eq!(cb.desc_head, 1, "head should advance to 1 after consume");
}
