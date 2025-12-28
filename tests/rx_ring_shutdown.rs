use std::time::Duration;

use ntx::rx_ring::{RxRing, RxRingConfig};

#[test]
fn wait_rx_async_wakes_on_shutdown() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    rt.block_on(async {
        let ring = RxRing::new(RxRingConfig {
            max_queue_depth: 8,
            lease_timeout: Duration::from_millis(5000),
        });

        let ring2 = ring.clone();
        let waiter = tokio::spawn(async move {
            // Large timeout; should return early due to shutdown.
            ring2.wait_rx_async(u32::MAX, u32::MAX, 60_000).await
        });

        // Give the waiter a moment to park on Notify.
        tokio::task::yield_now().await;

        ring.shutdown();

        // After shutdown, new enqueues must not make batches visible.
        ring.enqueue_batch(vec![1, 2, 3], vec![4, 5, 6]);
        assert!(
            ring.poll_rx(u32::MAX, u32::MAX).is_none(),
            "poll_rx should return None after shutdown"
        );

        let res = waiter.await.expect("wait task join");
        assert!(
            res.is_none(),
            "wait_rx_async should return None on shutdown"
        );
    });
}
