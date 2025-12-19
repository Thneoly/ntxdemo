//! End-to-end guestnet driver glue.
//!
//! This is the “wiring layer” between Packet I/O and the upper layers (Flow + Socket API).
//!
//! It is intentionally small and synchronous:
//! - no blocking (WouldBlock is surfaced)
//! - no packet parsing (parsing stays in Transport)
//! - no direct host ring access (only through `PacketIo` / `host_if`)

use crate::flow::FlowManager;
use crate::host_if::TxError;
use crate::packet_io::{GuestNetError, PacketIo};
use crate::socket_api::{PumpReport, SocketError, SocketTable};

/// What to submit to the host TX primitives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxSubmit {
    /// A fully-formed Ethernet frame.
    L2Frame(Vec<u8>),
    /// A fully-formed IPv4 packet (starting at IPv4 header, no Ethernet).
    L3Ipv4(Vec<u8>),
}

/// Diagnostics emitted by `drive_once`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DriveStats {
    pub packets_rx: u64,
    pub packets_bad_desc: u64,
    pub packets_on_packet_err: u64,
    pub socket_rx_full_drops: u64,

    pub tx_frames_sent: u64,
    pub tx_would_block: u64,
}

/// Detailed diagnostics emitted by `drive_once`.
///
/// This is intentionally cheap:
/// - no allocation
/// - `&SocketError` is borrowed from the live error value at the call site
#[derive(Debug)]
pub enum DriveReport<'e> {
    /// Periodic summary for the call.
    Stats(DriveStats),

    /// A packet caused `SocketTable::on_packet` to error.
    OnPacketError { err: &'e SocketError },

    /// Transport→socket pump encountered backpressure.
    Pump(PumpReport),

    /// TX submission hit backpressure.
    TxWouldBlock,
}

/// Run one non-blocking TX step.
///
/// Ordering:
/// 1) poll per-socket transport TX queues (encoding remains inside Transport)
/// 2) submit frames via HostIf TX primitive
pub fn drive_tx_once<'a, H, R>(
    pio: &mut PacketIo<'a, H>,
    _flows: &mut FlowManager,
    sockets: &mut SocketTable,
    mut report: R,
) -> Result<(), SocketError>
where
    H: crate::packet_io::HostIf,
    R: for<'e> FnMut(DriveReport<'e>),
{
    let mut stats = DriveStats::default();

    // MVP policy: scan socket IDs and drain their TX frames.
    // This is O(n) but keeps layering explicit; can be optimized later with readiness/eventing.
    let ids: Vec<crate::flow::SocketId> = sockets
        .debug_socket_ids_for_testing_and_pump()
        .into_iter()
        .collect();

    for id in ids {
        loop {
            let submit = match sockets.poll_tx(id) {
                Ok(s) => s,
                Err(SocketError::WouldBlock) => break,
                Err(SocketError::Unsupported) => break,
                Err(e) => return Err(e),
            };

            let r = match submit {
                TxSubmit::L2Frame(frame) => pio.host_tx_submit(&frame),
                TxSubmit::L3Ipv4(pkt) => pio.host_tx_submit_l3_ipv4(&pkt),
            };

            match r {
                Ok(()) => {
                    stats.tx_frames_sent = stats.tx_frames_sent.saturating_add(1);
                }
                Err(TxError::WouldBlock) => {
                    stats.tx_would_block = stats.tx_would_block.saturating_add(1);
                    report(DriveReport::TxWouldBlock);
                    break;
                }
                Err(TxError::Unsupported) => {
                    break;
                }
            }
        }
    }

    report(DriveReport::Stats(stats));
    Ok(())
}

/// Run one non-blocking RX+socket-pump step.
///
/// Call this from your outer event loop after receiving a `packet_event` and/or `socket_event`.
///
/// Ordering:
/// 1) drain packets from Host IF (PacketIo)
/// 2) pass packets into socket table (transport parsing happens internally)
/// 3) pump transport queues into socket rx buffers
pub fn drive_once<'a, H, R>(
    pio: &mut PacketIo<'a, H>,
    flows: &mut FlowManager,
    sockets: &mut SocketTable,
    mut report: R,
) -> Result<(), SocketError>
where
    H: crate::packet_io::HostIf,
    R: for<'e> FnMut(DriveReport<'e>),
{
    let mut stats = DriveStats::default();

    match pio.handle_packets(|pkt| {
        stats.packets_rx += 1;
        if let Err(e) = sockets.on_packet(flows, pkt) {
            stats.packets_on_packet_err += 1;
            report(DriveReport::OnPacketError { err: &e });
        }
    }) {
        Ok(()) => {}
        Err(GuestNetError::WouldBlock) => {}
        Err(GuestNetError::InvalidPacketDesc) => {
            // Treat invalid packet desc as a soft error.
            // In production we’d increment a counter and continue.
            stats.packets_bad_desc += 1;
        }
    }

    let pr = sockets.pump_transport_to_sockets_with_report(flows)?;
    stats.socket_rx_full_drops = pr.socket_rx_full_drops;

    report(DriveReport::Stats(stats));
    if pr != PumpReport::default() {
        report(DriveReport::Pump(pr));
    }
    Ok(())
}
