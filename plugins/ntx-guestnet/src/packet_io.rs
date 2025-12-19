//! Packet I/O layer (event-driven).
//!
//! Responsibilities:
//! - Wait for Host events (packet/timer/socket) via `host_if::poll_oneoff`.
//! - On `packet_event`, drain `host_if::poll_packet()` and convert to `PacketView` via shared mem.
//! - Forward `PacketView` upward (to FlowManager) via a callback.
//!
//! Non-responsibilities:
//! - Must NOT parse packet headers.
//! - Must NOT implement socket semantics.
//! - Must NOT access shared rings except through `host_if` primitives.

use crate::host_if::{self, Event, EventKind, PacketDesc, PacketView, SharedMem, TxError};

/// Host interface required by Packet I/O.
///
/// This is intentionally tiny and non-blocking.
///
/// In production WASM components this will be backed by host imports.
/// In tests we can provide a fake implementation.
pub trait HostIf {
    fn poll_packet(&mut self) -> Option<PacketDesc>;
    fn poll_oneoff(&mut self, interests: &[EventKind]) -> Vec<Event>;

    /// Submit a packet (L2 frame) for transmission.
    ///
    /// This is non-blocking and must not assume host sockets.
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), TxError>;

    /// Submit an IPv4 L3 packet for transmission.
    ///
    /// The payload starts at the IPv4 header (no Ethernet header).
    fn tx_submit_l3_ipv4(&mut self, packet: &[u8]) -> Result<(), TxError>;
}

/// Default HostIf implementation backed by the `host_if` module functions.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultHostIf;

impl HostIf for DefaultHostIf {
    fn poll_packet(&mut self) -> Option<PacketDesc> {
        host_if::poll_packet()
    }

    fn poll_oneoff(&mut self, interests: &[EventKind]) -> Vec<Event> {
        host_if::poll_oneoff(interests)
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        host_if::tx_submit(frame)
    }

    fn tx_submit_l3_ipv4(&mut self, packet: &[u8]) -> Result<(), TxError> {
        host_if::tx_submit_l3_ipv4(packet)
    }
}

/// Errors for non-blocking guestnet operations.
#[derive(Debug, thiserror::Error)]
pub enum GuestNetError {
    #[error("would block")]
    WouldBlock,

    #[error("invalid packet descriptor")]
    InvalidPacketDesc,
}

/// A tiny event-driven packet I/O runner.
///
/// This is intentionally *not* a global runtime; it is a library component that can be driven
/// by the scheduler's own control loop.
pub struct PacketIo<'a, H: HostIf = DefaultHostIf> {
    shm: &'a dyn SharedMem,
    host: H,
}

impl<'a> PacketIo<'a, DefaultHostIf> {
    pub fn new(shm: &'a dyn SharedMem) -> Self {
        Self {
            shm,
            host: DefaultHostIf,
        }
    }
}

impl<'a, H: HostIf> PacketIo<'a, H> {
    pub fn with_host(shm: &'a dyn SharedMem, host: H) -> Self {
        Self { shm, host }
    }

    /// Submit a TX frame via the underlying HostIf.
    #[inline]
    pub fn host_tx_submit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        self.host.tx_submit(frame)
    }

    /// Submit an IPv4 L3 packet via the underlying HostIf.
    #[inline]
    pub fn host_tx_submit_l3_ipv4(&mut self, packet: &[u8]) -> Result<(), TxError> {
        self.host.tx_submit_l3_ipv4(packet)
    }

    /// Expose the underlying host implementation for tests/integration glue.
    ///
    /// This is intentionally a narrow escape hatch; production code should not rely on it.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Drain all currently-available RX packets once.
    ///
    /// - Never blocks.
    /// - Returns `WouldBlock` if no packets are available.
    pub fn handle_packets<F>(&mut self, mut on_packet: F) -> Result<(), GuestNetError>
    where
        F: FnMut(PacketView<'_>),
    {
        let mut any = false;
        while let Some(desc) = self.host.poll_packet() {
            any = true;
            let view = host_if::packet_view_from_desc(self.shm, desc)
                .ok_or(GuestNetError::InvalidPacketDesc)?;
            on_packet(view);
        }

        if any {
            Ok(())
        } else {
            Err(GuestNetError::WouldBlock)
        }
    }

    /// A minimal event loop skeleton strictly following the required model.
    ///
    /// The callbacks correspond to the upper layers:
    /// - `handle_packets` will call into FlowManager
    /// - `handle_timers` will call into Transport(s)
    /// - `handle_socket_io` will flush Socket ↔ Transport glue
    ///
    /// Notes:
    /// - `poll_oneoff` is currently a stub; the host integration will provide real events.
    /// - This loop is provided as a reference; production code can embed the same pattern.
    pub fn run_loop<HP, HT, HS>(
        &mut self,
        mut handle_packets: HP,
        mut handle_timers: HT,
        mut handle_socket_io: HS,
    ) -> !
    where
        HP: FnMut() + 'static,
        HT: FnMut() + 'static,
        HS: FnMut() + 'static,
    {
        loop {
            let _events =
                self.host
                    .poll_oneoff(&[EventKind::Packet, EventKind::Timer, EventKind::Socket]);

            // Required ordering.
            handle_packets();
            handle_timers();
            handle_socket_io();
        }
    }
}

/// Packet metadata only (no payload).
///
/// This is useful when upper layers want to key off flow_hash/proto without touching bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketMeta {
    pub l3_proto: u8,
    pub l4_proto: u8,
    pub flow_hash: u64,
}

impl From<PacketDesc> for PacketMeta {
    fn from(d: PacketDesc) -> Self {
        Self {
            l3_proto: d.l3_proto,
            l4_proto: d.l4_proto,
            flow_hash: d.flow_hash,
        }
    }
}
