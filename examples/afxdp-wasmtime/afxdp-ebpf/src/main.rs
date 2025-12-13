#![no_std]
#![no_main]
#![allow(static_mut_refs)]

use aya_ebpf::{
    bindings::xdp_action,
    macros::{map, xdp},
    maps::PerCpuArray,
    maps::XskMap,
    programs::XdpContext,
};

/// XSK map: key = RX queue id, value = bound XSK socket.
/// Userspace will populate this with `bpf` object loading.
#[map(name = "XSKS")]
static mut XSKS: XskMap = XskMap::with_max_entries(64, 0);

/// Debug counters: per-CPU array.
///
/// Index meanings:
/// 0 = xdp program hit count
/// 1 = redirect ok count
/// 2 = redirect err count (XSKS missing or redirect failure)
/// 3 = pass count
/// 4 = xdp_redirect (action == XDP_REDIRECT)
/// 5 = xdp_drop (action == XDP_DROP)
/// 6 = xdp_aborted (action == XDP_ABORTED)
/// 7 = xdp_tx (action == XDP_TX)
#[map(name = "XDP_STATS")]
static mut XDP_STATS: PerCpuArray<u64> = PerCpuArray::with_max_entries(8, 0);

#[inline(always)]
fn bump(idx: u32) {
    // Best-effort counters. If the map isn't accessible for some reason, ignore.
    unsafe {
        if let Some(v) = XDP_STATS.get_ptr_mut(idx) {
            *v += 1;
        }
    }
}

#[xdp]
pub fn xdp_redirect_to_xsk(ctx: XdpContext) -> u32 {
    bump(0);
    match unsafe { try_redirect(ctx) } {
        Ok(ret) => ret,
        Err(_) => xdp_action::XDP_ABORTED,
    }
}

unsafe fn try_redirect(ctx: XdpContext) -> Result<u32, u32> {
    // DEBUG knob (compile-time): if you ever suspect the XDP program isn't being hit,
    // build with `--features xdp-abort` to make any received packet on the interface
    // get dropped with XDP_ABORTED (easy to observe via `ping` failing).
    #[cfg(feature = "xdp-abort")]
    {
        let _ = ctx;
        return Ok(xdp_action::XDP_ABORTED);
    }

    // Redirect to AF_XDP socket bound to this RX queue.
    // If no socket is registered for the queue, redirect() returns an error.
    let qid = ctx.rx_queue_index();
    // `XSKS` is a `static mut`; avoid creating a shared reference (Rust 2024).
    match unsafe { XSKS.redirect(qid, 0) } {
        Ok(action) => {
            bump(1);
            // Record what action we actually returned.
            match action {
                xdp_action::XDP_REDIRECT => bump(4),
                xdp_action::XDP_DROP => bump(5),
                xdp_action::XDP_ABORTED => bump(6),
                xdp_action::XDP_TX => bump(7),
                _ => {}
            }
            Ok(action)
        }
        Err(_) => {
            bump(2);
            bump(3);
            Ok(xdp_action::XDP_PASS)
        }
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
