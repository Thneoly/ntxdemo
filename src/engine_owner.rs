use tracing::info;

use crate::app_config::WasmEngineMode;
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
    mode: WasmEngineMode,
) -> tokio::sync::mpsc::Sender<EngineMsg> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<EngineMsg>(1024);

    // Shared RX ring provider used by the guest imports.
    let rx_ring = engine.rx_ring();

    // Single owner task. It runs either:
    // - guest `run()` forever (legacy)
    // - or host-driven init+step loop (stepper)
    // while also servicing RX batch deliveries.
    tokio::spawn(async move {
        let mut shutting_down = false;

        match mode {
            WasmEngineMode::Run => {
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

                info!(target: "ntx::engine-owner", "engine owner started (mode=run); entering message loop");
                while let Some(msg) = rx.recv().await {
                    match msg {
                        EngineMsg::RxBatch { desc, payload } => {
                            if !shutting_down {
                                let batch_id = ((desc.len() as u64) << 32) ^ (payload.len() as u64);
                                tracing::info!(
                                    target: "ntx::engine-owner",
                                    batch_id,
                                    desc_len = desc.len(),
                                    payload_len = payload.len(),
                                    "received RxBatch; enqueue into rx-ring"
                                );
                                rx_ring.enqueue_batch(desc, payload);
                            }
                        }
                        EngineMsg::Shutdown => {
                            tracing::info!(
                                target: "ntx::engine-owner",
                                "received shutdown; rx-ring shutdown + stop accepting RX"
                            );
                            rx_ring.shutdown();
                            shutting_down = true;
                        }
                    }
                }

                let _ = run_join.await;
            }
            WasmEngineMode::Stepper => {
                let mut step_engine = engine;
                if !step_engine.supports_stepper() {
                    tracing::error!(
                        target: "ntx::engine-owner",
                        "engine configured for stepper mode, but component has no scheduler-stepper export"
                    );
                    return;
                }

                if let Err(e) = step_engine.init_stepper(config_dir).await {
                    tracing::error!(
                        target: "ntx::engine-owner",
                        error = %e,
                        error_dbg = ?e,
                        "stepper init failed"
                    );
                    return;
                }

                info!(target: "ntx::engine-owner", "engine owner started (mode=stepper)");

                // Drive step() repeatedly; let guest do bounded waits (timeout_ms).
                let mut next_wait = std::time::Duration::from_millis(0);
                loop {
                    tokio::select! {
                        msg = rx.recv() => {
                            let Some(msg) = msg else {
                                break;
                            };
                            match msg {
                                EngineMsg::RxBatch { desc, payload } => {
                                    if !shutting_down {
                                        rx_ring.enqueue_batch(desc, payload);
                                        // New RX arrived; step ASAP.
                                        next_wait = std::time::Duration::from_millis(0);
                                    }
                                }
                                EngineMsg::Shutdown => {
                                    tracing::info!(target: "ntx::engine-owner", "received shutdown; request-stop + rx-ring shutdown");
                                    rx_ring.shutdown();
                                    shutting_down = true;
                                    let _ = step_engine.request_stop_stepper().await;
                                    next_wait = std::time::Duration::from_millis(0);
                                }
                            }
                        }
                        _ = tokio::time::sleep(next_wait) => {
                            let args = crate::wasm_engine::SchedulerStepArgs {
                                max_events: 64,
                                max_dispatch: 64,
                                timeout_ms: 10,
                                now_ms: None,
                            };

                            match step_engine.step_stepper(args).await {
                                Ok(res) => {
                                    let wait = res.suggested_wait_ms as u64;
                                    next_wait = std::time::Duration::from_millis(wait);
                                    if res.did_work {
                                        next_wait = std::time::Duration::from_millis(0);
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(target: "ntx::engine-owner", error = %e, error_dbg=?e, "stepper step failed");
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    tx
}
