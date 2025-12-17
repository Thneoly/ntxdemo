use std::any::Any;
use std::collections::HashMap;

use super::layer::{LayerId, LayerInstance};

/// Extra information a decoder can optionally provide to help the registry choose
/// the next layer (Scapy-like `bind_layers`).
///
/// This intentionally stays tiny and copyable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindKey {
    UdpDstPort(u16),
    UdpSrcPort(u16),
    TcpDstPort(u16),
    TcpSrcPort(u16),
}

/// A decoder returns:
/// - the decoded layer instance
/// - bytes consumed
/// - `next_hint`: a direct next layer id chosen by the layer itself (fast path)
/// - `bind_key`: optional data to allow registry-driven binding rules
pub type DecodeFn =
    fn(&[u8]) -> Result<(LayerInstance, usize, Option<LayerId>, Option<BindKey>), String>;

pub type EncodeFn = fn(&dyn Any, payload: &[u8], out: &mut Vec<u8>) -> Result<(), String>;

type BindRule = (LayerId, BindKey);

/// Tunnel extractor: given the just-decoded layer and its payload slice,
/// return the first `LayerId` to parse for the inner packet.
///
/// Example: VXLAN payload is an inner Ethernet frame => returns `Some(LayerId::Ether)`.
pub type TunnelFn = fn(layer: &dyn Any, payload: &[u8]) -> Result<Option<LayerId>, String>;

/// Runtime registry that maps `LayerId` <-> decoder/encoder glue.
///
/// This is the key to "protocols not being hardcoded": adding a new protocol is
/// purely "implement Layer + register".
pub struct LayerRegistry {
    decoders: HashMap<LayerId, DecodeFn>,
    encoders: HashMap<LayerId, EncodeFn>,
    bindings: HashMap<BindRule, LayerId>,
    tunnels: HashMap<LayerId, TunnelFn>,
}

impl Default for LayerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LayerRegistry {
    pub fn new() -> Self {
        Self {
            decoders: HashMap::new(),
            encoders: HashMap::new(),
            bindings: HashMap::new(),
            tunnels: HashMap::new(),
        }
    }

    pub fn register_decoder(&mut self, id: LayerId, f: DecodeFn) {
        self.decoders.insert(id, f);
    }

    pub fn register_encoder(&mut self, id: LayerId, f: EncodeFn) {
        self.encoders.insert(id, f);
    }

    /// Register a Scapy-like binding rule: when `parent` produces `key`, interpret
    /// payload as the returned `LayerId`.
    pub fn bind(&mut self, parent: LayerId, key: BindKey, next: LayerId) {
        self.bindings.insert((parent, key), next);
    }

    /// Register a tunnel extractor for a given layer.
    ///
    /// This is the "structural" part of the hybrid approach: a layer can be marked
    /// as producing an inner packet, and the extractor determines the inner first-layer.
    pub fn register_tunnel(&mut self, id: LayerId, f: TunnelFn) {
        self.tunnels.insert(id, f);
    }

    pub fn decode<'a>(
        &self,
        id: LayerId,
        input: &'a [u8],
    ) -> Result<(LayerInstance, usize, Option<LayerId>, Option<BindKey>), String> {
        let f = self
            .decoders
            .get(&id)
            .ok_or_else(|| format!("unknown layer decoder: {:?}", id))?;
        f(input)
    }

    /// Resolve a registry-driven next layer using `parent` and the provided `BindKey`.
    pub fn resolve_binding(&self, parent: LayerId, key: BindKey) -> Option<LayerId> {
        self.bindings.get(&(parent, key)).copied()
    }

    /// If `id` is a tunnel-capable layer, determine the inner packet's first layer.
    pub fn tunnel_next(
        &self,
        id: LayerId,
        layer_any: &dyn Any,
        payload: &[u8],
    ) -> Result<Option<LayerId>, String> {
        let Some(f) = self.tunnels.get(&id) else {
            return Ok(None);
        };
        f(layer_any, payload)
    }

    pub fn encode(
        &self,
        id: LayerId,
        layer_any: &dyn Any,
        payload: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<(), String> {
        let f = self
            .encoders
            .get(&id)
            .ok_or_else(|| format!("unknown layer encoder: {:?}", id))?;
        f(layer_any, payload, out)
    }
}
