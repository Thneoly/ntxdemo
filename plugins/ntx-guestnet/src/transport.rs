//! Transport layer.
//!
//! Hard rule: all packet parsing/generation lives in this module (or its submodules).
//! Nothing above Transport may parse bytes.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use crate::flow::{FlowKey, FlowManager, SocketId, TransportProto};
use crate::host_if::PacketView;

use ntx_network::socket::ConnTableConfig;
use ntx_network::socket::udp::Table as ConnTable;
use ntx_network::stack::{
    LayerId, PacketContext, ParsedPacket, default_registry, parse_packet_with_ctx,
};

/// Transport-layer errors.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("would block")]
    WouldBlock,

    #[error("malformed packet: {0}")]
    MalformedPacket(MalformedPacketReason),

    #[error("unsupported")]
    Unsupported,
}

/// Structured reason for packet malformation.
///
/// This intentionally avoids heap allocation so it can safely cross layers and
/// later map cleanly to WIT enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedPacketReason {
    Ethernet,
    Ipv4,
    Udp,
}

impl core::fmt::Display for MalformedPacketReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            MalformedPacketReason::Ethernet => "ethernet",
            MalformedPacketReason::Ipv4 => "ipv4",
            MalformedPacketReason::Udp => "udp",
        };
        f.write_str(s)
    }
}

/// Socket-facing receive result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecvDatagram {
    pub src: ([u8; 4], u16),
    pub dst: ([u8; 4], u16),
    pub payload: Vec<u8>,
}

/// Socket-facing send request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendDatagram<'a> {
    pub src: ([u8; 4], u16),
    pub dst: ([u8; 4], u16),
    pub payload: &'a [u8],
}

/// RAW IPv4 send request.
///
/// This is intentionally separate from `SendDatagram` so RAW semantics don't overload
/// UDP/TCP address fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRawIpv4<'a> {
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
    pub proto: u8,
    pub payload: &'a [u8],
}

/// Transport trait (exact layering boundary).
///
/// Contract:
/// - `on_packet` parses and classifies bytes and updates flow state.
/// - `poll_recv` produces socket-level receives without blocking.
/// - `send` enqueues a send request (may return WouldBlock on backpressure).
/// - `poll_timer` advances any internal timers.
///
/// NOTE: in this strict architecture, Transport does not access host rings directly.
/// Packet I/O eventually hands packets to FlowManager+Transport.
pub trait Transport {
    fn on_packet(
        &mut self,
        flows: &mut FlowManager,
        pkt: PacketView<'_>,
    ) -> Result<(), TransportError>;

    fn poll_recv(&mut self, socket: SocketId) -> Result<RecvDatagram, TransportError>;

    fn send(&mut self, socket: SocketId, req: SendDatagram<'_>) -> Result<usize, TransportError>;

    fn poll_timer(&mut self, flows: &mut FlowManager, now_tick: u64) -> Result<(), TransportError>;
}

/// Minimal UDP transport.
///
/// RX: parses Ethernet+IPv4+UDP, classifies to `FlowKey`, looks up bound socket via FlowManager,
/// and enqueues a datagram for that socket.
///
/// TX: currently just buffers datagrams in-memory (placeholder for future packet generation).
#[derive(Debug, Default)]
pub struct UdpTransport {
    rxq: HashMap<SocketId, VecDeque<RecvDatagram>>,
    txq: HashMap<SocketId, VecDeque<RecvDatagram>>,

    /// Per-socket queue capacity to force WouldBlock semantics.
    max_queue_len: usize,

    /// Cache for per-flow header templates.
    ///
    /// NOTE: It remains in UdpTransport mainly for lower-level tests and future reuse.
    /// The Socket API layer may also maintain its own connection cache for 'connect' semantics.
    conns: ConnTable,
}

/// RAW (L3 IPv4) transport.
///
/// TX contract: `send()` accepts an IPv4 payload provided as bytes plus metadata:
/// - `req.src.0` is the local IPv4 address
/// - `req.dst.0` is the remote IPv4 address
/// - `req.src.1` carries the IPv4 `protocol` number (0-255)
///
/// Transport encodes the IPv4 header and enqueues a complete IPv4 packet which the driver submits
/// via host L3 primitive (`tx_submit_l3_ipv4`).
///
/// RX is intentionally left unimplemented for now; once enabled it will parse IPv4 and deliver
/// raw packets to bound sockets without exposing parsing above Transport.
#[derive(Debug, Default)]
pub struct RawTransport {
    txq: HashMap<SocketId, VecDeque<Vec<u8>>>,
    max_queue_len: usize,
}

impl RawTransport {
    pub fn new(max_queue_len: usize) -> Self {
        Self {
            txq: HashMap::new(),
            max_queue_len,
        }
    }

    fn txq_for(&mut self, s: SocketId) -> &mut VecDeque<Vec<u8>> {
        self.txq.entry(s).or_default()
    }

    pub fn poll_tx_ipv4(&mut self, socket: SocketId) -> Result<Vec<u8>, TransportError> {
        let q = self.txq_for(socket);
        q.pop_front().ok_or(TransportError::WouldBlock)
    }

    pub fn send_raw_ipv4(
        &mut self,
        socket: SocketId,
        req: SendRawIpv4<'_>,
    ) -> Result<usize, TransportError> {
        let max = self.max_queue_len;
        let q = self.txq_for(socket);
        if q.len() >= max {
            return Err(TransportError::WouldBlock);
        }

        let payload = req.payload;
        let pkt_len = ntx_network::packet::headers::Ipv4Header::MIN_LEN + payload.len();
        let mut out = vec![0u8; pkt_len];

        let ip = ntx_network::packet::headers::Ipv4Header {
            src: ntx_network::packet::headers::Ipv4Addr(req.src_ip),
            dst: ntx_network::packet::headers::Ipv4Addr(req.dst_ip),
            protocol: req.proto,
            ttl: 64,
            identification: 0,
            flags_fragment: 0,
        };

        ip.encode(
            &mut out[..ntx_network::packet::headers::Ipv4Header::MIN_LEN],
            payload.len(),
            0,
        )
        .map_err(|_| TransportError::MalformedPacket(MalformedPacketReason::Ipv4))?;

        out[ntx_network::packet::headers::Ipv4Header::MIN_LEN..].copy_from_slice(payload);
        q.push_back(out);
        Ok(payload.len())
    }
}

impl Transport for RawTransport {
    fn on_packet(
        &mut self,
        _flows: &mut FlowManager,
        _pkt: PacketView<'_>,
    ) -> Result<(), TransportError> {
        Err(TransportError::Unsupported)
    }

    fn poll_recv(&mut self, _socket: SocketId) -> Result<RecvDatagram, TransportError> {
        Err(TransportError::Unsupported)
    }

    fn send(&mut self, socket: SocketId, req: SendDatagram<'_>) -> Result<usize, TransportError> {
        let proto_u16 = req.src.1;
        let proto: u8 = u8::try_from(proto_u16)
            .map_err(|_| TransportError::MalformedPacket(MalformedPacketReason::Ipv4))?;

        self.send_raw_ipv4(
            socket,
            SendRawIpv4 {
                src_ip: req.src.0,
                dst_ip: req.dst.0,
                proto,
                payload: req.payload,
            },
        )
    }

    fn poll_timer(
        &mut self,
        _flows: &mut FlowManager,
        _now_tick: u64,
    ) -> Result<(), TransportError> {
        Ok(())
    }
}

/// Placeholder Ethernet (L2) transport.
#[derive(Debug, Default)]
pub struct EthTransport;

impl Transport for EthTransport {
    fn on_packet(
        &mut self,
        _flows: &mut FlowManager,
        _pkt: PacketView<'_>,
    ) -> Result<(), TransportError> {
        Err(TransportError::Unsupported)
    }

    fn poll_recv(&mut self, _socket: SocketId) -> Result<RecvDatagram, TransportError> {
        Err(TransportError::Unsupported)
    }

    fn send(&mut self, _socket: SocketId, _req: SendDatagram<'_>) -> Result<usize, TransportError> {
        Err(TransportError::Unsupported)
    }

    fn poll_timer(
        &mut self,
        _flows: &mut FlowManager,
        _now_tick: u64,
    ) -> Result<(), TransportError> {
        Ok(())
    }
}

impl UdpTransport {
    pub fn new(max_queue_len: usize) -> Self {
        Self {
            rxq: HashMap::new(),
            txq: HashMap::new(),
            max_queue_len,
            conns: ConnTable::new(ConnTableConfig {
                max_entries: 4096,
                ttl: Some(Duration::from_secs(60)),
            }),
        }
    }

    fn rxq_for(&mut self, s: SocketId) -> &mut VecDeque<RecvDatagram> {
        self.rxq.entry(s).or_default()
    }

    fn txq_for(&mut self, s: SocketId) -> &mut VecDeque<RecvDatagram> {
        self.txq.entry(s).or_default()
    }

    pub fn conns_mut(&mut self) -> &mut ConnTable {
        &mut self.conns
    }

    /// Poll one buffered outgoing UDP datagram for a socket.
    ///
    /// This is intentionally internal for now; Packet I/O/Host IF TX wiring comes later.
    pub fn poll_tx(&mut self, socket: SocketId) -> Result<RecvDatagram, TransportError> {
        let q = self.txq_for(socket);
        q.pop_front().ok_or(TransportError::WouldBlock)
    }

    /// Encode and poll one outgoing UDP frame (L2) for a socket.
    ///
    /// This keeps packet generation strictly inside Transport.
    ///
    /// `peer_mac/local_mac` are provided by the caller (socket layer / neighbor module).
    pub fn poll_tx_frame_with_l2(
        &mut self,
        socket: SocketId,
        peer_mac: ntx_network::packet::headers::MacAddr,
        local_mac: ntx_network::packet::headers::MacAddr,
    ) -> Result<Vec<u8>, TransportError> {
        let dg = self.poll_tx(socket)?;

        // Use the shared network socket layer to cache per-flow templates.
        let conn = self.conns.connect(
            ntx_network::packet::headers::Ipv4Addr(dg.dst.0),
            dg.dst.1,
            ntx_network::packet::headers::Ipv4Addr(dg.src.0),
            dg.src.1,
            peer_mac,
            local_mac,
            64,
        );
        let frame = conn
            .build_reply(&dg.payload)
            .map_err(|_| TransportError::MalformedPacket(MalformedPacketReason::Udp))?;

        Ok(frame.bytes)
    }

    /// Back-compat shim used by older tests/callers.
    ///
    /// TODO: remove once socket layer/driver always passes real L2 addressing.
    pub fn poll_tx_frame(&mut self, socket: SocketId) -> Result<Vec<u8>, TransportError> {
        let peer_mac = ntx_network::packet::headers::MacAddr([6, 7, 8, 9, 10, 11]);
        let local_mac = ntx_network::packet::headers::MacAddr([0, 1, 2, 3, 4, 5]);
        self.poll_tx_frame_with_l2(socket, peer_mac, local_mac)
    }
}

impl Transport for UdpTransport {
    fn on_packet(
        &mut self,
        flows: &mut FlowManager,
        pkt: PacketView<'_>,
    ) -> Result<(), TransportError> {
        // Parse via the shared network parsing pipeline, so guestnet RX stays aligned with
        // the core `ntx-network` stack.
        let bytes = pkt.as_bytes();

        // TODO: in future, thread a shared registry through the host/driver so we don't
        // rebuild it per packet. This is acceptable for MVP.
        let registry = default_registry();
        let ctx = PacketContext::default();
        let (layers, payload) = parse_packet_with_ctx(bytes, LayerId::Ether, &registry, &ctx)
            .map_err(|_| TransportError::MalformedPacket(MalformedPacketReason::Ethernet))?;

        let parsed = ParsedPacket { layers, payload };

        // Keep timing consistent across protocol demux (UDP/TCP/...) and avoid
        // scattered `Instant::now()` calls.
        let now = std::time::Instant::now();

        let Some(ip) = parsed.get::<ntx_network::stack::layers::Ipv4>() else {
            return Err(TransportError::MalformedPacket(MalformedPacketReason::Ipv4));
        };
        if ip.proto != 17 {
            return Err(TransportError::Unsupported);
        }
        let Some(udp) = parsed.get::<ntx_network::stack::layers::Udp>() else {
            return Err(TransportError::MalformedPacket(MalformedPacketReason::Udp));
        };

        // Refresh reverse-path template cache (server-style “reply along the way it came in”).
        //
        // For RX we *do* have the L2 addresses from the Ethernet header, so use them directly.
        let Some(eth) = parsed.get::<ntx_network::stack::layers::Ether>() else {
            return Err(TransportError::MalformedPacket(
                MalformedPacketReason::Ethernet,
            ));
        };
        let my_mac = eth.dst;
        let my_ip = ntx_network::packet::headers::Ipv4Addr(ip.dst.0);

        // Build minimal reusable contexts once per packet.
        let time = ntx_network::socket::TimeContext::with_now(now);
        let local = ntx_network::socket::LocalIdentity::new(my_mac, my_ip);
        let udp_rxctx = ntx_network::socket::UdpRxContext::new(time, local, udp.dst_port);
        let _ = self
            .conns
            .on_rx(&parsed, &udp_rxctx)
            .map_err(|_| TransportError::MalformedPacket(MalformedPacketReason::Udp))?;

        let key = FlowKey {
            proto: TransportProto::Udp,
            src_ip: ip.src.0,
            dst_ip: ip.dst.0,
            src_port: udp.src_port,
            dst_port: udp.dst_port,
        };

        // Touch flow table for last_seen.
        let _entry = flows.lookup_or_create(key);

        // Deliver to bound socket (if any). Unbound flows are currently dropped.
        let Some(socket) = flows.socket_for_inbound_flow(&key) else {
            return Ok(());
        };

        let max = self.max_queue_len;
        let q = self.rxq_for(socket);
        if q.len() >= max {
            return Err(TransportError::WouldBlock);
        }

        q.push_back(RecvDatagram {
            src: (key.src_ip, key.src_port),
            dst: (key.dst_ip, key.dst_port),
            payload: parsed.payload().to_vec(),
        });
        Ok(())
    }

    fn poll_recv(&mut self, socket: SocketId) -> Result<RecvDatagram, TransportError> {
        let q = self.rxq_for(socket);
        q.pop_front().ok_or(TransportError::WouldBlock)
    }

    fn send(&mut self, socket: SocketId, req: SendDatagram<'_>) -> Result<usize, TransportError> {
        let max = self.max_queue_len;
        let q = self.txq_for(socket);
        if q.len() >= max {
            return Err(TransportError::WouldBlock);
        }

        q.push_back(RecvDatagram {
            src: req.src,
            dst: req.dst,
            payload: req.payload.to_vec(),
        });
        Ok(req.payload.len())
    }

    fn poll_timer(
        &mut self,
        _flows: &mut FlowManager,
        _now_tick: u64,
    ) -> Result<(), TransportError> {
        // UDP has no timers in this skeleton.
        Ok(())
    }
}

/// Placeholder TCP transport.
///
/// This exists to cement the layering boundary; TCP FSM comes later.
#[derive(Debug, Default)]
pub struct TcpTransport;

impl Transport for TcpTransport {
    fn on_packet(
        &mut self,
        _flows: &mut FlowManager,
        _pkt: PacketView<'_>,
    ) -> Result<(), TransportError> {
        Err(TransportError::Unsupported)
    }

    fn poll_recv(&mut self, _socket: SocketId) -> Result<RecvDatagram, TransportError> {
        Err(TransportError::Unsupported)
    }

    fn send(&mut self, _socket: SocketId, _req: SendDatagram<'_>) -> Result<usize, TransportError> {
        Err(TransportError::Unsupported)
    }

    fn poll_timer(
        &mut self,
        _flows: &mut FlowManager,
        _now_tick: u64,
    ) -> Result<(), TransportError> {
        Ok(())
    }
}
