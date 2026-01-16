use std::time::Instant;

use crate::io::rx_decode;

/// Best-effort RX pumping when threads are unavailable.
///
/// Contract:
/// - Must never panic.
/// - Must be non-blocking (0ms wait) so it doesn't stall the main event loop.
/// - Always release the handle.
pub(crate) fn pump_rx_once_nonblocking() -> u32 {
    struct PumpStats {
        polls: u64,
        timeouts: u64,
        batches: u64,
        read_errors: u64,
        decode_errors: u64,
        last_desc_len: u32,
        last_payload_len: u32,
        last_seq: u64,
        last_log: Instant,
    }

    static STATS: once_cell::sync::Lazy<std::sync::Mutex<PumpStats>> =
        once_cell::sync::Lazy::new(|| {
            std::sync::Mutex::new(PumpStats {
                polls: 0,
                timeouts: 0,
                batches: 0,
                read_errors: 0,
                decode_errors: 0,
                last_desc_len: 0,
                last_payload_len: 0,
                last_seq: 0,
                last_log: Instant::now(),
            })
        });

    // Stop flag
    if crate::runtime::runtime_state::RUNTIME
        .lock()
        .map(|rt| rt.stop)
        .unwrap_or(false)
    {
        return 0;
    }

    // Non-blocking poll: timeout 0ms.
    let batch = crate::bindmod::ntx::host::rx_ring::wait_rx(64 * 1024, 256 * 1024, 0);
    let Some(batch) = batch else {
        let mut s = STATS.lock().unwrap();
        s.timeouts = s.timeouts.saturating_add(1);
        return 0;
    };

    println!(
        "[scheduler] pump_rx_once_nonblocking: got batch seq {}",
        batch.seq
    );
    {
        let mut s = STATS.lock().unwrap();
        s.batches = s.batches.saturating_add(1);
        s.last_desc_len = batch.desc_len;
        s.last_payload_len = batch.payload_len;
        s.last_seq = batch.seq;
    }

    let handle = batch.handle;

    let desc_mem = match crate::bindmod::ntx::host::rx_ring::read_desc(handle, 0, batch.desc_len) {
        Ok(v) => v,
        Err(e) => {
            println!("[scheduler] rx-ring read-desc failed: {e}");
            let mut s = STATS.lock().unwrap();
            s.read_errors = s.read_errors.saturating_add(1);
            let _ = crate::bindmod::ntx::host::rx_ring::release(handle);
            return 1;
        }
    };

    let payload_mem =
        match crate::bindmod::ntx::host::rx_ring::read_payload(handle, 0, batch.payload_len) {
            Ok(v) => v,
            Err(e) => {
                println!("[scheduler] rx-ring read-payload failed: {e}");
                let mut s = STATS.lock().unwrap();
                s.read_errors = s.read_errors.saturating_add(1);
                let _ = crate::bindmod::ntx::host::rx_ring::release(handle);
                return 1;
            }
        };

    let drained = rx_decode::drain_rx_ring(desc_mem, payload_mem);
    if drained == 0 {
        let mut s = STATS.lock().unwrap();
        s.decode_errors = s.decode_errors.saturating_add(1);
    }

    let _ = crate::bindmod::ntx::host::rx_ring::release(handle);

    1
}
