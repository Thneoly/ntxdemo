use std::any::Any;

use crate::abr;
use crate::{Ipv4Addr, MacAddr};

/// A stable identifier for a protocol layer.
///
/// Keep this small/copyable so it can be used as a key for registries and scripting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerId {
    Ether,
    Arp,
    Ipv4,
    Udp,
    Tcp,
    Vxlan,
    /// Indicates "bytes with no further structure".
    Payload,
}

/// Post-decode filtering result.
///
/// This lets each layer decide whether to accept the decoded header, drop the packet,
/// or stop parsing further ("poison") even though the structure is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptResult {
    /// Accept this layer and continue parsing as usual.
    Accept,
    /// Drop the packet as irrelevant noise (e.g. not destined to us).
    Drop,
    /// The structure is valid, but we should stop parsing further layers.
    ///
    /// Use this when the packet is "not ours" at this layer but higher layers should
    /// not be interpreted (e.g. IPv4 dst isn't an owned IP; treat as non-fatal).
    Poison,
}

/// Per-packet context for `Layer::accept()` decisions.
///
/// Keep this small/copy-friendly; it is passed by reference during parsing.
#[derive(Debug, Clone, Default)]
pub struct PacketContext {
    /// If set: accept L2 frames destined to this MAC (or broadcast/multicast).
    pub iface_mac: Option<MacAddr>,
    /// Active Binding Resource (ABR) view.
    ///
    /// This is the dataplane "source of truth" for whether a packet is destined to
    /// a locally-bound resource (IPs/ports/etc.).
    ///
    /// If `None`, layers should fall back to permissive behavior (accept-all).
    pub abr: Option<std::sync::Arc<abr::ResourceView>>,

    /// Deprecated: previously used as a "local/bound IPv4 set".
    ///
    /// Kept temporarily to reduce churn in callers/tests. Prefer `abr.ipv4`.
    #[allow(dead_code)]
    pub local_ipv4: Vec<Ipv4Addr>,
}

/// Protocol layer with self-description: it knows how to decode itself, and decide next.
///
/// Design goals:
/// - No generic nesting (no `Ether<Ipv4<Udp<...>>>`).
/// - No big `match` in a monolithic parser.
/// - Next-layer decision is data-driven (per-layer `next_layer()`).
pub trait Layer<'a>: Sized + 'a {
    const ID: LayerId;

    /// Decode `Self` from `input`.
    ///
    /// Returns `(layer, bytes_consumed)`.
    fn decode(input: &'a [u8]) -> Result<(Self, usize), String>;

    /// Encode this layer into `out`, with `payload` provided by the lower layer.
    ///
    /// This writes into `out` instead of returning a new `Vec` to allow:
    /// - zero-copy friendly patterns
    /// - DPDK/AF_XDP integration
    /// - WASM linear memory friendly writes
    fn encode(&self, payload: &[u8], out: &mut Vec<u8>);

    /// Decide whether this decoded layer should be accepted.
    ///
    /// Default: accept everything.
    fn accept(&self, _ctx: &PacketContext) -> AcceptResult {
        AcceptResult::Accept
    }

    /// Decide what the next layer id is.
    fn next_layer(&self) -> Option<LayerId>;
}

/// Type-erased layer instance.
///
/// This enables Scapy-style access patterns: `pkt[Ipv4].src`, etc.
///
/// Note: the stored `inner` is owned (not a reference) for simplicity and object safety.
#[derive(Debug)]
pub struct LayerInstance {
    pub id: LayerId,
    pub inner: Box<dyn Any + Send + Sync>,
}

impl LayerInstance {
    /// Create a type-erased layer instance.
    #[inline]
    pub fn new<T>(id: LayerId, layer: T) -> Self
    where
        T: Any + Send + Sync + 'static,
    {
        Self {
            id,
            inner: Box::new(layer),
        }
    }

    /// Convenience: create a layer instance from a concrete layer type that implements `Layer`.
    #[inline]
    pub fn layer<'a, L>(layer: L) -> Self
    where
        L: Layer<'a> + Any + Send + Sync + 'static,
    {
        Self::new(L::ID, layer)
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }
}

/// Convenience constructors for built-in layers.
///
/// These are purely ergonomic helpers to keep call sites readable.
pub mod li {
    use super::{LayerId, LayerInstance};

    #[inline]
    pub fn ether(layer: crate::packet::layers::Ether) -> LayerInstance {
        LayerInstance::new(LayerId::Ether, layer)
    }

    #[inline]
    pub fn arp(layer: crate::packet::layers::Arp) -> LayerInstance {
        LayerInstance::new(LayerId::Arp, layer)
    }

    #[inline]
    pub fn ipv4(layer: crate::packet::layers::Ipv4) -> LayerInstance {
        LayerInstance::new(LayerId::Ipv4, layer)
    }

    #[inline]
    pub fn udp(layer: crate::packet::layers::Udp) -> LayerInstance {
        LayerInstance::new(LayerId::Udp, layer)
    }

    #[inline]
    pub fn tcp(layer: crate::packet::layers::Tcp) -> LayerInstance {
        LayerInstance::new(LayerId::Tcp, layer)
    }

    #[inline]
    pub fn vxlan(layer: crate::packet::layers::Vxlan) -> LayerInstance {
        LayerInstance::new(LayerId::Vxlan, layer)
    }
}

impl PacketContext {
    /// Build a context that carries a stable ABR snapshot.
    ///
    /// This is the recommended dataplane pattern when you want `ctx` to carry the view:
    /// load ABR once per batch (or per RX loop tick) and use it for all packets in that batch.
    pub fn with_abr_view(view: std::sync::Arc<crate::abr::ResourceView>) -> Self {
        Self {
            iface_mac: None,
            abr: Some(view),
            local_ipv4: Vec::new(),
        }
    }
}
