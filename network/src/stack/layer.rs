use std::any::Any;

/// A stable identifier for a protocol layer.
///
/// Keep this small/copyable so it can be used as a key for registries and scripting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerId {
    Ether,
    Ipv4,
    Udp,
    Tcp,
    Vxlan,
    /// Indicates "bytes with no further structure".
    Payload,
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
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }
}
