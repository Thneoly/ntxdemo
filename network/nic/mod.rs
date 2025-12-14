mod afpacket;
mod tpacketv3;

use std::time::Duration;

/// A minimal NIC backend abstraction used by examples/tools.
///
/// Contract:
/// - `send()` transmits a full L2 frame.
/// - `recv()` is allowed to block.
/// - `recv_nonblocking()` returns `Ok(None)` when no frame is available.
/// - `poll_readable()` waits until the NIC is readable (or times out).
#[allow(dead_code)]
pub trait Nic {
    fn ifindex(&self) -> i32;
    fn ifname(&self) -> &str;
    fn iface_mac(&self) -> Option<[u8; 6]>;

    fn send(&self, frame: &[u8]) -> anyhow::Result<usize>;
    fn recv(&mut self, buf: &mut [u8]) -> anyhow::Result<usize>;
    fn recv_nonblocking(&mut self, buf: &mut [u8]) -> anyhow::Result<Option<usize>>;

    /// Block until the NIC becomes readable.
    ///
    /// Returns `Ok(true)` if readable, `Ok(false)` on timeout.
    fn poll_readable(&self, timeout: Option<Duration>) -> anyhow::Result<bool>;

    /// Optional per-received-frame packet type hint.
    ///
    /// For AF_PACKET sockets this corresponds to `sockaddr_ll.sll_pkttype`.
    /// Backends that don't have this concept should return None.
    fn last_pkttype(&self) -> Option<u8> {
        None
    }
}

#[allow(unused_imports)]
pub use afpacket::AfPacketNic;

#[allow(unused_imports)]
pub use afpacket::AfPacketDgramNic;

#[allow(unused_imports)]
pub use tpacketv3::TpacketV3Nic;
