//! Flow Manager layer.
//!
//! This layer is intentionally *transport-agnostic*.
//!
//! Contract:
//! - Inputs: a `FlowKey` computed by the Transport layer (after parsing the packet bytes).
//! - Outputs: stable per-flow state (`FlowEntry`) and per-flow routing/binding metadata.
//! - Must NOT parse packet headers.
//! - Must NOT block.
//! - Must NOT access shared rings or shared memory payloads.

use core::hash::{Hash, Hasher};
use std::collections::HashMap;

/// IPv4 endpoint (address + port).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EndpointV4 {
    pub ip: [u8; 4],
    pub port: u16,
}

/// Transport family used for flow classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportProto {
    Udp,
    Tcp,

    /// Raw IPv4 packets (L3).
    Raw,

    /// Raw Ethernet frames (L2).
    Eth,
}

/// Socket bind key (two-level model).
///
/// This is the *socket-level* lookup key that maps to a `SocketId`.
///
/// - For UDP, `remote` is typically `None` unless the socket is "connected".
/// - `local.ip` may be a wildcard/default (e.g. 0.0.0.0) depending on higher-layer policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketBindKey {
    pub proto: TransportProto,
    pub local: EndpointV4,
    pub remote: Option<EndpointV4>,
}

/// IPv4 5-tuple key.
///
/// Notes:
/// - Kept minimal and copy-friendly.
/// - A `flow_hash` may exist from Host IF, but we still need a canonical key for correctness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlowKey {
    pub proto: TransportProto,
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
    pub src_port: u16,
    pub dst_port: u16,
}

impl FlowKey {
    /// Convert a 5-tuple flow into a socket-level bind lookup key.
    ///
    /// Direction: inbound packet (src -> dst).
    #[inline]
    pub fn inbound_bind_key(&self) -> SocketBindKey {
        SocketBindKey {
            proto: self.proto,
            local: EndpointV4 {
                ip: self.dst_ip,
                port: self.dst_port,
            },
            remote: Some(EndpointV4 {
                ip: self.src_ip,
                port: self.src_port,
            }),
        }
    }
}

/// Identifies a socket binding (opaque to FlowManager).
///
/// Socket semantics live above FlowManager and below Application.
/// FlowManager only tracks which flow is associated with which socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketId(pub u32);

/// Per-flow entry managed by FlowManager.
#[derive(Debug, Clone)]
pub struct FlowEntry {
    pub key: FlowKey,

    /// LRU/GC hinting. This avoids tying flow lifecycle to transport specifics.
    pub last_seen_tick: u64,
}

impl FlowEntry {
    fn new(key: FlowKey, now_tick: u64) -> Self {
        Self {
            key,
            last_seen_tick: now_tick,
        }
    }
}

/// A small flow table.
///
/// This is a simple HashMap-based skeleton; we can optimize later (e.g., slab + hash to index).
#[derive(Debug, Default)]
pub struct FlowManager {
    flows: HashMap<FlowKey, FlowEntry>,

    /// Socket bind table.
    ///
    /// Kept here (below Transport) so it can be consulted during RX classification
    /// without pulling socket semantics down into host_if/packet_io.
    binds: HashMap<SocketBindKey, SocketId>,

    /// A monotonic tick supplied by the driving loop.
    now_tick: u64,
}

impl FlowManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the manager's idea of time.
    ///
    /// The scheduler loop (or a higher layer) decides ticks; FlowManager just stores them.
    pub fn set_now_tick(&mut self, now_tick: u64) {
        self.now_tick = now_tick;
    }

    /// Lookup a flow entry, inserting a new one if absent.
    ///
    /// Returns a mutable reference so callers can update per-flow state.
    pub fn lookup_or_create(&mut self, key: FlowKey) -> &mut FlowEntry {
        let now = self.now_tick;
        let entry = self
            .flows
            .entry(key)
            .or_insert_with(|| FlowEntry::new(key, now));
        entry.last_seen_tick = now;
        entry
    }

    /// Add/replace a socket binding.
    pub fn bind_socket(&mut self, key: SocketBindKey, socket: SocketId) {
        self.binds.insert(key, socket);
    }

    /// Remove a socket binding.
    pub fn unbind_socket(&mut self, key: &SocketBindKey) {
        self.binds.remove(key);
    }

    /// Resolve an inbound flow to a socket binding.
    ///
    /// Lookup order (UDP-friendly):
    /// 1) exact 4-tuple bind (local + remote)
    /// 2) local-only bind (local + remote None)
    pub fn socket_for_inbound_flow(&self, flow: &FlowKey) -> Option<SocketId> {
        let exact = flow.inbound_bind_key();
        if let Some(s) = self.binds.get(&exact) {
            return Some(*s);
        }

        let local_only = SocketBindKey {
            proto: exact.proto,
            local: exact.local,
            remote: None,
        };
        self.binds.get(&local_only).copied()
    }

    pub fn len(&self) -> usize {
        self.flows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }
}

/// A stable hash helper for FlowKey.
///
/// This is useful if we want to maintain per-flow queues in another structure.
pub fn flow_key_hash(key: &FlowKey) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}
