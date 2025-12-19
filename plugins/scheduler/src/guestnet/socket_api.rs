//! Socket API layer (library abstraction).
//!
//! Hard rules:
//! - No packet parsing here.
//! - No shared ring/shared memory access here.
//! - No blocking: backpressure is explicit via WouldBlock.
//!
//! This module is the Rust-side implementation skeleton that will later be exposed through
//! WIT (`scheduler:net/socket-api`) by a dedicated component.

use std::collections::{HashMap, VecDeque};

use crate::guestnet::driver::TxSubmit;
use crate::guestnet::flow::{EndpointV4, FlowManager, SocketBindKey, SocketId, TransportProto};
use crate::guestnet::transport::{
    MalformedPacketReason, RecvDatagram, SendDatagram, Transport, TransportError, UdpTransport,
};

/// Socket kinds (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketKind {
    Datagram,
    Stream,
}

/// Socket error surface.
#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    #[error("would block")]
    WouldBlock,

    #[error("invalid state")]
    InvalidState,

    #[error("address in use")]
    AddrInUse,

    #[error("invalid argument")]
    InvalidArgument,

    #[error("unsupported")]
    Unsupported,

    #[error("malformed packet: {0}")]
    MalformedPacket(MalformedPacketReason),
}

impl From<TransportError> for SocketError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::WouldBlock => SocketError::WouldBlock,
            TransportError::MalformedPacket(r) => SocketError::MalformedPacket(r),
            TransportError::Unsupported => SocketError::Unsupported,
        }
    }
}

/// Reports non-fatal conditions encountered while pumping transport → socket buffers.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PumpReport {
    /// Number of datagrams that could not be moved into socket rx due to socket backpressure.
    pub socket_rx_full_drops: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketReadiness {
    pub readable: bool,
    pub writable: bool,
    pub has_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportSel {
    Udp,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Init,
    Bound,
    Connected,
    Listening,
    Closed,
}

#[derive(Debug, Clone)]
pub struct Socket {
    pub id: SocketId,
    pub kind: SocketKind,
    pub proto: TransportProto,
    pub state: SocketState,

    pub local: Option<EndpointV4>,
    pub remote: Option<EndpointV4>,

    /// Optional local-only bind key (proto+local).
    ///
    /// For UDP, this corresponds to a "wildcard remote" binding.
    pub bind_key_local_only: Option<SocketBindKey>,

    /// Optional connected bind key (proto+local+remote).
    pub bind_key_connected: Option<SocketBindKey>,

    /// Socket-level receive buffer.
    ///
    /// Transport already does parsing; this is purely socket semantics (queueing bytes).
    rx: VecDeque<RecvDatagram>,

    /// Max queued datagrams to enforce backpressure.
    rx_max: usize,
}

impl Socket {
    fn new(id: SocketId, kind: SocketKind, proto: TransportProto) -> Self {
        Self {
            id,
            kind,
            proto,
            state: SocketState::Init,
            local: None,
            remote: None,
            bind_key_local_only: None,
            bind_key_connected: None,
            rx: VecDeque::new(),
            rx_max: 128,
        }
    }

    fn readiness(&self) -> SocketReadiness {
        SocketReadiness {
            readable: !self.rx.is_empty(),
            writable: self.state != SocketState::Closed,
            has_error: false,
        }
    }
}

/// Owns sockets and the transports.
///
/// For now we only implement UDP end-to-end.
#[derive(Debug, Default)]
pub struct SocketTable {
    next_id: u32,
    sockets: HashMap<SocketId, Socket>,

    udp: UdpTransport,
}

impl SocketTable {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            sockets: HashMap::new(),
            // Keep the transport queue small-ish; socket rx buffer is what apps see.
            udp: UdpTransport::new(64),
        }
    }

    fn alloc_id(&mut self) -> SocketId {
        let id = SocketId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    pub fn socket(
        &mut self,
        kind: SocketKind,
        proto: TransportProto,
    ) -> Result<SocketId, SocketError> {
        match (kind, proto) {
            (SocketKind::Datagram, TransportProto::Udp) => {}
            (SocketKind::Datagram, TransportProto::Raw) => {}
            (SocketKind::Datagram, TransportProto::Eth) => {}
            (SocketKind::Stream, TransportProto::Tcp) => {
                // Not implemented yet, but keep API consistent.
                return Err(SocketError::Unsupported);
            }
            _ => return Err(SocketError::InvalidArgument),
        }

        let id = self.alloc_id();
        self.sockets.insert(id, Socket::new(id, kind, proto));
        Ok(id)
    }

    pub fn close(&mut self, flows: &mut FlowManager, id: SocketId) -> Result<(), SocketError> {
        let Some(sock) = self.sockets.get_mut(&id) else {
            return Err(SocketError::InvalidArgument);
        };

        if let Some(k) = sock.bind_key_connected {
            flows.unbind_socket(&k);
        }
        if let Some(k) = sock.bind_key_local_only {
            flows.unbind_socket(&k);
        }

        sock.state = SocketState::Closed;
        Ok(())
    }

    pub fn bind(
        &mut self,
        flows: &mut FlowManager,
        id: SocketId,
        local: EndpointV4,
    ) -> Result<(), SocketError> {
        let Some(sock) = self.sockets.get_mut(&id) else {
            return Err(SocketError::InvalidArgument);
        };
        if sock.state == SocketState::Closed {
            return Err(SocketError::InvalidState);
        }
        if sock.kind != SocketKind::Datagram || sock.proto != TransportProto::Udp {
            return Err(SocketError::Unsupported);
        }

        let key = SocketBindKey {
            proto: sock.proto,
            local,
            remote: None,
        };

        // NOTE: we don't yet enforce AddrInUse globally (needs reverse map SocketBindKey->SocketId).
        // We simply overwrite for now.
        flows.bind_socket(key, id);

        sock.local = Some(local);
        sock.remote = None;
        sock.bind_key_local_only = Some(key);
        sock.bind_key_connected = None;
        sock.state = SocketState::Bound;
        Ok(())
    }

    pub fn connect(
        &mut self,
        flows: &mut FlowManager,
        id: SocketId,
        remote: EndpointV4,
    ) -> Result<(), SocketError> {
        let Some(sock) = self.sockets.get_mut(&id) else {
            return Err(SocketError::InvalidArgument);
        };
        if sock.state == SocketState::Closed {
            return Err(SocketError::InvalidState);
        }
        if sock.kind != SocketKind::Datagram || sock.proto != TransportProto::Udp {
            return Err(SocketError::Unsupported);
        }

        let Some(local) = sock.local else {
            // Require explicit bind for now (keeps policy decisions out of this skeleton).
            return Err(SocketError::InvalidState);
        };

        let connected_key = SocketBindKey {
            proto: sock.proto,
            local,
            remote: Some(remote),
        };

        flows.bind_socket(connected_key, id);

        sock.remote = Some(remote);
        sock.bind_key_connected = Some(connected_key);
        sock.state = SocketState::Connected;
        Ok(())
    }

    /// Pull any ready packets from the transports into socket rx buffers.
    ///
    /// This is intended to be called from the scheduler loop after `Transport::on_packet` has been
    /// given a chance to enqueue transport-level receives.
    pub fn pump_transport_to_sockets(
        &mut self,
        flows: &mut FlowManager,
    ) -> Result<(), SocketError> {
        let mut _report = PumpReport::default();

        // UDP: for every socket, try to drain transport rx into socket rx.
        // This keeps WouldBlock semantics and avoids a global scan later we can optimize.
        let ids: Vec<SocketId> = self.sockets.keys().copied().collect();
        for id in ids {
            let Some(sock) = self.sockets.get_mut(&id) else {
                continue;
            };
            if sock.proto != TransportProto::Udp {
                continue;
            }
            loop {
                match self.udp.poll_recv(id) {
                    Ok(dg) => {
                        if sock.rx.len() >= sock.rx_max {
                            // Backpressure at socket layer.
                            // Put it back into transport tx? For now, stop.
                            _report.socket_rx_full_drops =
                                _report.socket_rx_full_drops.saturating_add(1);
                            break;
                        }
                        // Optional remote filtering: if connected, only accept that remote.
                        if let Some(r) = sock.remote {
                            if dg.src != (r.ip, r.port) {
                                continue;
                            }
                        }

                        // NOTE: Flow lifecycle/last_seen is intentionally owned by Transport.
                        // Socket pump must not mutate flow state.
                        let _ = flows;

                        sock.rx.push_back(dg);
                    }
                    Err(TransportError::WouldBlock) => break,
                    Err(e) => return Err(SocketError::from(e)),
                }
            }
        }
        Ok(())
    }

    /// Same as `pump_transport_to_sockets`, but also returns a lightweight pressure report.
    pub fn pump_transport_to_sockets_with_report(
        &mut self,
        flows: &mut FlowManager,
    ) -> Result<PumpReport, SocketError> {
        let mut report = PumpReport::default();

        let ids: Vec<SocketId> = self.sockets.keys().copied().collect();
        for id in ids {
            let Some(sock) = self.sockets.get_mut(&id) else {
                continue;
            };
            if sock.proto != TransportProto::Udp {
                continue;
            }
            loop {
                match self.udp.poll_recv(id) {
                    Ok(dg) => {
                        if sock.rx.len() >= sock.rx_max {
                            report.socket_rx_full_drops =
                                report.socket_rx_full_drops.saturating_add(1);
                            break;
                        }
                        if let Some(r) = sock.remote {
                            if dg.src != (r.ip, r.port) {
                                continue;
                            }
                        }

                        let _ = flows;
                        sock.rx.push_back(dg);
                    }
                    Err(TransportError::WouldBlock) => break,
                    Err(e) => return Err(SocketError::from(e)),
                }
            }
        }

        Ok(report)
    }

    pub fn on_packet(
        &mut self,
        flows: &mut FlowManager,
        pkt: crate::guestnet::host_if::PacketView<'_>,
    ) -> Result<(), SocketError> {
        self.udp.on_packet(flows, pkt).map_err(SocketError::from)
    }

    pub fn recv(&mut self, id: SocketId, max_len: usize) -> Result<Vec<u8>, SocketError> {
        let Some(sock) = self.sockets.get_mut(&id) else {
            return Err(SocketError::InvalidArgument);
        };
        if sock.state == SocketState::Closed {
            return Err(SocketError::InvalidState);
        }

        let Some(dg) = sock.rx.pop_front() else {
            return Err(SocketError::WouldBlock);
        };

        let n = dg.payload.len().min(max_len);
        Ok(dg.payload[..n].to_vec())
    }

    pub fn send(&mut self, id: SocketId, data: &[u8]) -> Result<usize, SocketError> {
        let Some(sock) = self.sockets.get(&id) else {
            return Err(SocketError::InvalidArgument);
        };
        if sock.state == SocketState::Closed {
            return Err(SocketError::InvalidState);
        }
        if sock.proto != TransportProto::Udp {
            return Err(SocketError::Unsupported);
        }

        let (local, remote) = match (sock.local, sock.remote) {
            (Some(l), Some(r)) => (l, r),
            _ => return Err(SocketError::InvalidState),
        };

        let req = SendDatagram {
            src: (local.ip, local.port),
            dst: (remote.ip, remote.port),
            payload: data,
        };
        self.udp.send(id, req).map_err(SocketError::from)
    }

    /// Poll one outgoing L2 frame produced by transports for a given socket.
    ///
    /// This keeps packet generation inside Transport while allowing the driver to submit bytes
    /// to Host IF TX primitives.
    pub fn poll_tx(&mut self, id: SocketId) -> Result<TxSubmit, SocketError> {
        let Some(sock) = self.sockets.get(&id) else {
            return Err(SocketError::InvalidArgument);
        };
        if sock.state == SocketState::Closed {
            return Err(SocketError::InvalidState);
        }

        match sock.proto {
            TransportProto::Udp => self
                .udp
                .poll_tx_frame(id)
                .map(TxSubmit::L2Frame)
                .map_err(SocketError::from),
            TransportProto::Tcp => Err(SocketError::Unsupported),
            TransportProto::Raw => {
                // TODO: implement raw TX as L3 IPv4 packet bytes.
                Err(SocketError::Unsupported)
            }
            TransportProto::Eth => Err(SocketError::Unsupported),
        }
    }

    /// Compatibility shim: poll only L2 frames.
    pub fn poll_tx_frame(&mut self, id: SocketId) -> Result<Vec<u8>, SocketError> {
        match self.poll_tx(id)? {
            TxSubmit::L2Frame(f) => Ok(f),
            TxSubmit::L3Ipv4(_) => Err(SocketError::Unsupported),
        }
    }

    pub fn poll(&self, id: SocketId) -> Result<SocketReadiness, SocketError> {
        let Some(sock) = self.sockets.get(&id) else {
            return Err(SocketError::InvalidArgument);
        };
        Ok(sock.readiness())
    }

    /// Returns current socket IDs.
    ///
    /// This exists to support the MVP TX pump (`drive_tx_once`) in unit tests.
    /// A production-quality implementation should use readiness/event signals instead of scanning.
    pub fn debug_socket_ids_for_testing_and_pump(&self) -> Vec<SocketId> {
        self.sockets.keys().copied().collect()
    }
}
