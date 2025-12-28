use std::time::Duration;

use ntx::engine_owner::EngineMsg;
use ntx::rx_ring::{RxRing, RxRingConfig};

#[test]
fn engine_owner_shutdown_msg_wakes_wait_rx() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    rt.block_on(async {
        // Use the same ring semantics as the host import provider.
        let ring = RxRing::new(RxRingConfig {
            max_queue_depth: 8,
            lease_timeout: Duration::from_millis(5000),
        });

        // A minimal “owner-like” task: receives EngineMsg and triggers ring.shutdown().
        // This isolates the requirement we care about (shutdown wakes waiters) without
        // needing to run a real wasm component.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<EngineMsg>(8);
        let ring_owner_side = ring.clone();
        let owner = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if matches!(msg, EngineMsg::Shutdown) {
                    ring_owner_side.shutdown();
                    break;
                }
            }
        });

        // Waiter task should park on Notify and get woken by shutdown.
        let ring_wait_side = ring.clone();
        let waiter = tokio::spawn(async move {
            ring_wait_side
                .wait_rx_async(u32::MAX, u32::MAX, 60_000)
                .await
        });

        tokio::task::yield_now().await;

        tx.send(EngineMsg::Shutdown).await.expect("send shutdown");

        let res = waiter.await.expect("wait task join");
        assert!(
            res.is_none(),
            "wait_rx_async should return None on shutdown"
        );

        owner.await.expect("owner join");
    });
}
