use tracing::info;

use crate::wasm_engine::{ComponentEngine, EngineError};

/// Messages sent to the engine owner.
#[derive(Debug)]
pub enum EngineMsg {
    RxBatch {
        desc: Vec<u8>,
        payload: Vec<u8>,
    },
    /// Request the engine owner to begin shutdown.
    ///
    /// Semantics (end-state):
    /// - Wake any guest `wait-rx` import calls by calling `rx_ring.shutdown()`.
    /// - Stop accepting new RX batches.
    /// - The owner task may then exit once its channel is closed.
    Shutdown,
}

/// Spawn the engine owner task.
///
/// Contract:
/// - The owner is the **only** place that mutates/uses the `ComponentEngine`.
/// - Other tasks communicate via `tokio::mpsc`.
/// - RX batches are enqueued into the engine's host `rx_ring`.
pub fn spawn_engine_owner(
    engine: ComponentEngine,
    config_dir: String,
) -> tokio::sync::mpsc::Sender<EngineMsg> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<EngineMsg>(1024);

    // Shared RX ring provider used by the guest imports.
    let rx_ring = engine.rx_ring();

    // Single owner task. It runs the guest `run()` forever, while also servicing
    // RX batch deliveries.
    tokio::spawn(async move {
        // Drive guest run() in a separate task so we can still receive messages.
        let mut run_engine = engine;
        let run_join = tokio::spawn(async move {
            if let Err(e) = run_engine.run(config_dir).await {
                tracing::error!(
                    target: "ntx::engine-owner",
                    error = %e,
                    error_dbg = ?e,
                    "guest scheduler run() exited with error"
                );
                Err::<(), EngineError>(e)
            } else {
                Ok(())
            }
        });

        // NOTE: currently the guest `run()` is expected to never return.
        // We treat `run_join` completion as fatal/error.
        let mut shutting_down = false;
        info!(target: "ntx::engine-owner", "engine owner started; entering message loop");
        while let Some(msg) = rx.recv().await {
            info!(
                target: "ntx::engine-owner",
                "received engine message: {:?}",
                msg
            );
            match msg {
                EngineMsg::RxBatch { desc, payload } => {
                    if !shutting_down {
                        let batch_id = {
                            // Derive a stable-ish id from lengths (good enough for log correlation
                            // when combined with timestamps). We avoid global atomics here.
                            ((desc.len() as u64) << 32) ^ (payload.len() as u64)
                        };
                        tracing::info!(
                            target: "ntx::engine-owner",
                            batch_id,
                            desc_len = desc.len(),
                            payload_len = payload.len(),
                            "received RxBatch; enqueue into rx-ring"
                        );
                        rx_ring.enqueue_batch(desc, payload);

                        tracing::debug!(
                            target: "ntx::engine-owner",
                            batch_id,
                            queue_depth = rx_ring.queue_depth(),
                            inflight = rx_ring.inflight_handles(),
                            bytes_in_queue = rx_ring.bytes_in_queue(),
                            "rx-ring state after enqueue"
                        );
                    }
                }
                EngineMsg::Shutdown => {
                    // Wake any guest waiters so `wait-rx` returns quickly.
                    tracing::info!(
                        target: "ntx::engine-owner",
                        "received shutdown; rx-ring shutdown + stop accepting RX"
                    );
                    rx_ring.shutdown();
                    // After shutdown, stop accepting new batches (end-state semantics).
                    shutting_down = true;
                }
            }
        }

        let _ = run_join.await;
    });

    tx
}
