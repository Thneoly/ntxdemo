//! Host interface primitives (the ONLY permitted bridge to the host datapath).
//!
//! Rules enforced by design:
//! - No socket concepts appear here.
//! - No packet parsing happens here.
//! - No implicit waiting/blocking; host events are surfaced via `poll_oneoff`.
//! - RX payload must be accessed by (offset,len) into shared memory; **no Vec copies**.

use core::ops::Range;

/// Descriptor for a received packet in the shared RX ring.
///
/// This is the only metadata the Guest datapath gets from the Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketDesc {
    pub buf_offset: u32,
    pub len: u32,
    pub l3_proto: u8,
    pub l4_proto: u8,
    pub flow_hash: u64,
}

/// A borrowed view of a packet payload stored in Host-provided shared memory.
///
/// Important: this type intentionally does *not* expose a `Vec<u8>`.
#[derive(Debug, Clone, Copy)]
pub struct PacketView<'a> {
    pub desc: PacketDesc,
    bytes: &'a [u8],
}

impl<'a> PacketView<'a> {
    #[inline]
    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
}

/// Shared memory abstraction.
///
/// In the real integration this will be backed by the host-provided shared memory mapping.
/// For now it is a minimal borrowing API to support no-copy PacketView.
pub trait SharedMem {
    /// Borrow a read-only slice from [offset .. offset+len).
    ///
    /// This must be a *borrow* into the shared memory mapping (no allocation/copy).
    fn get_range(&self, range: Range<u32>) -> Option<&[u8]>;
}

/// Events that can be waited on using the WASI-style poller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Packet,
    Timer,
    Socket,
}

/// Result from `poll_oneoff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub kind: EventKind,
}

/// Transmit a frame to the host datapath.
///
/// This is a *packet primitive* (not a socket primitive): the guest provides a fully-formed
/// L2 frame as bytes.
///
/// Contract:
/// - Non-blocking: must return immediately.
/// - Backpressure is explicit via `WouldBlock`.
/// - The host is free to copy the bytes into its TX ring; guest must not assume zero-copy.
#[derive(Debug, thiserror::Error)]
pub enum TxError {
    #[error("would block")]
    WouldBlock,

    #[error("unsupported")]
    Unsupported,
}

/// Submit a single IPv4 L3 packet for transmission.
///
/// This is a packet primitive: the guest provides a fully-formed IPv4 packet (starting at the
/// IPv4 header, no Ethernet header).
///
/// Contract:
/// - Non-blocking: must return immediately.
/// - Backpressure is explicit via `WouldBlock`.
/// - The host is responsible for L2 resolution/encapsulation.
///
/// NOTE: not implemented in this crate yet. It will be provided via component imports.
#[allow(unused_variables)]
pub fn tx_submit_l3_ipv4(_packet: &[u8]) -> Result<(), TxError> {
    Err(TxError::Unsupported)
}

/// Submit a single L2 frame for transmission.
///
/// NOTE: not implemented in this crate yet. It will be provided via component imports.
#[allow(unused_variables)]
pub fn tx_submit(_frame: &[u8]) -> Result<(), TxError> {
    Err(TxError::Unsupported)
}

/// Poll an RX packet from the host.
///
/// Returns a descriptor pointing into shared memory.
///
/// NOTE: not implemented in this crate yet. It will be provided via component imports.
#[allow(unused_variables)]
pub fn poll_packet() -> Option<PacketDesc> {
    // Integration point:
    // - In real WASM, this will be an imported function from the host.
    // - It must be non-blocking.
    None
}

/// Poll for one or more events (packet/timer/socket).
///
/// Events carry no data; data must be polled separately (e.g. `poll_packet`).
#[allow(unused_variables)]
pub fn poll_oneoff(_interests: &[EventKind]) -> Vec<Event> {
    // Integration point:
    // - In real WASM, delegate to `wasi::io::poll::poll_oneoff` or a custom host import.
    // - Must not block forever in library code; the top-level runner controls the loop.
    Vec::new()
}

/// Helper: build a PacketView from a PacketDesc using shared memory.
///
/// This enforces the “no-copy payload” constraint: only shared memory slices are returned.
#[inline]
pub fn packet_view_from_desc<'a>(
    shm: &'a dyn SharedMem,
    desc: PacketDesc,
) -> Option<PacketView<'a>> {
    let start = desc.buf_offset;
    let end = desc.buf_offset.checked_add(desc.len)?;
    let bytes = shm.get_range(start..end)?;
    Some(PacketView { desc, bytes })
}
