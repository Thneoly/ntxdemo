use super::layer::{LayerId, LayerInstance, PacketContext};
use super::layers::register_all;
use super::parser::{parse_packet, parse_packet_with_ctx};
use super::registry::LayerRegistry;

/// Create a registry pre-populated with the built-in layers.
///
/// Prefer reusing a registry across packets to avoid per-packet allocations.
pub fn default_registry() -> LayerRegistry {
    let mut r = LayerRegistry::new();
    register_all(&mut r);
    r
}

/// A reply frame buffer.
#[derive(Debug, Clone)]
pub struct ReplyFrame {
    pub bytes: Vec<u8>,
}

/// The result of a handler.
#[derive(Debug, Clone)]
pub enum Action {
    /// Ignore this packet.
    Pass,
    /// Send a reply frame.
    Reply(ReplyFrame),
}

/// Stateless packet handler operating on a parsed chain.
pub trait PacketHandler {
    fn handle(&mut self, pkt: &ParsedPacket<'_>) -> anyhow::Result<Action>;
}

/// Parsed packet view: extracted typed layers + payload slice.
#[derive(Debug)]
pub struct ParsedPacket<'a> {
    /// Extracted typed layers.
    ///
    /// Kept public for back-compat with older call sites that constructed this
    /// view directly from `(layers, payload)` produced by the parser.
    pub layers: Vec<LayerInstance>,

    /// Remaining payload bytes after the last decoded layer.
    ///
    /// Kept public for back-compat; prefer using [`ParsedPacket::payload()`].
    pub payload: &'a [u8],
}

impl<'a> ParsedPacket<'a> {
    pub fn layers(&self) -> &[LayerInstance] {
        &self.layers
    }

    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.layers.iter().find_map(|l| l.downcast_ref::<T>())
    }

    pub fn find(&self, id: LayerId) -> Option<&LayerInstance> {
        self.layers.iter().find(|l| l.id == id)
    }
}

/// A small orchestrator that decodes a frame and runs a set of handlers.
///
/// The first handler returning `Action::Reply` wins.
pub struct Pipeline {
    handlers: Vec<Box<dyn PacketHandler>>,
    registry: LayerRegistry,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
            registry: default_registry(),
        }
    }

    /// Construct a pipeline with a custom registry (e.g. with extra protocol layers).
    pub fn with_registry(registry: LayerRegistry) -> Self {
        Self {
            handlers: Vec::new(),
            registry,
        }
    }

    pub fn add_handler<H: PacketHandler + 'static>(&mut self, h: H) {
        self.handlers.push(Box::new(h));
    }

    /// Borrow the registry used by this pipeline.
    pub fn registry(&self) -> &LayerRegistry {
        &self.registry
    }

    /// Replace the registry (e.g. after registering additional layers).
    pub fn set_registry(&mut self, registry: LayerRegistry) {
        self.registry = registry;
    }

    pub fn process(&mut self, frame: &[u8]) -> anyhow::Result<Action> {
        let (layers, payload) =
            parse_packet(frame, LayerId::Ether, &self.registry).map_err(anyhow::Error::msg)?;
        let pkt = ParsedPacket { layers, payload };

        for h in self.handlers.iter_mut() {
            match h.handle(&pkt)? {
                Action::Pass => continue,
                r @ Action::Reply(_) => return Ok(r),
            }
        }
        Ok(Action::Pass)
    }

    /// Like [`Pipeline::process`], but uses the provided [`PacketContext`] for per-layer
    /// `accept()` decisions.
    ///
    /// Intended usage: the caller loads a stable ABR snapshot once per batch and stores it
    /// in `ctx.abr`, then calls this for each packet in the batch.
    pub fn process_with_ctx(
        &mut self,
        frame: &[u8],
        ctx: &PacketContext,
    ) -> anyhow::Result<Action> {
        let (layers, payload) = parse_packet_with_ctx(frame, LayerId::Ether, &self.registry, ctx)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let pkt = ParsedPacket { layers, payload };

        for h in self.handlers.iter_mut() {
            match h.handle(&pkt)? {
                Action::Pass => continue,
                r @ Action::Reply(_) => return Ok(r),
            }
        }
        Ok(Action::Pass)
    }
}
