use crate::{ETH_TYPE_IPV4, EthernetHeader, Ipv4Header, MacAddr, UdpHeader};

use super::layer::{LayerId, LayerInstance, PacketContext};
use super::layers::{Ether, Ipv4, Udp, register_all};
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

/// A minimal UDP echo handler.
///
/// Filter:
/// - Ethernet dst is iface mac OR broadcast
/// - IPv4 + UDP
/// - udp.dst_port == listen_port
#[derive(Debug, Clone)]
pub struct UdpEchoHandler {
    pub listen_port: u16,
    pub iface_mac: MacAddr,
    pub verbose: bool,
}

impl PacketHandler for UdpEchoHandler {
    fn handle(&mut self, pkt: &ParsedPacket<'_>) -> anyhow::Result<Action> {
        let Some(eth) = pkt.get::<Ether>() else {
            return Ok(Action::Pass);
        };

        // L2 filter
        if !eth.dst.is_broadcast() && eth.dst != self.iface_mac {
            return Ok(Action::Pass);
        }

        let Some(ip) = pkt.get::<Ipv4>() else {
            return Ok(Action::Pass);
        };
        if ip.proto != 17 {
            return Ok(Action::Pass);
        }

        let Some(udp) = pkt.get::<Udp>() else {
            return Ok(Action::Pass);
        };
        if udp.dst_port != self.listen_port {
            return Ok(Action::Pass);
        }

        if self.verbose {
            eprintln!(
                "echo hit: eth {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}; ip {}.{}.{}.{}:{} -> {}.{}.{}.{}:{}; payload_len={}",
                eth.src.0[0],
                eth.src.0[1],
                eth.src.0[2],
                eth.src.0[3],
                eth.src.0[4],
                eth.src.0[5],
                eth.dst.0[0],
                eth.dst.0[1],
                eth.dst.0[2],
                eth.dst.0[3],
                eth.dst.0[4],
                eth.dst.0[5],
                ip.src.0[0],
                ip.src.0[1],
                ip.src.0[2],
                ip.src.0[3],
                udp.src_port,
                ip.dst.0[0],
                ip.dst.0[1],
                ip.dst.0[2],
                ip.dst.0[3],
                udp.dst_port,
                pkt.payload().len()
            );
        }

        Ok(Action::Reply(build_udp_reply(pkt, self.iface_mac)?))
    }
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

/// Build an IPv4/UDP echo reply, swapping MAC/IP/port.
///
/// Requires the parsed packet to contain Ether + Ipv4 + Udp.
pub fn build_udp_reply(pkt: &ParsedPacket<'_>, iface_mac: MacAddr) -> anyhow::Result<ReplyFrame> {
    let eth = pkt
        .get::<Ether>()
        .ok_or_else(|| anyhow::anyhow!("missing ether"))?;
    let ip = pkt
        .get::<Ipv4>()
        .ok_or_else(|| anyhow::anyhow!("missing ipv4"))?;
    let udp = pkt
        .get::<Udp>()
        .ok_or_else(|| anyhow::anyhow!("missing udp"))?;

    let payload = pkt.payload();

    let reply_eth = EthernetHeader {
        dst: eth.src,
        src: iface_mac,
        ethertype: ETH_TYPE_IPV4,
    };

    let reply_ip = Ipv4Header {
        src: ip.dst,
        dst: ip.src,
        protocol: 17,
        ttl: ip.ttl,
        identification: ip.identification,
        flags_fragment: ip.flags_fragment,
    };

    let reply_udp = UdpHeader {
        src_port: udp.dst_port,
        dst_port: udp.src_port,
    };

    let eth_len = EthernetHeader::LEN;
    let ip_len = Ipv4Header::MIN_LEN;
    let udp_len = UdpHeader::LEN;

    let mut bytes = vec![0u8; eth_len + ip_len + udp_len + payload.len()];

    reply_eth.encode(&mut bytes[..eth_len])?;
    reply_ip.encode(
        &mut bytes[eth_len..eth_len + ip_len],
        udp_len + payload.len(),
        0,
    )?;

    let udp_off = eth_len + ip_len;
    reply_udp.encode(
        &mut bytes[udp_off..udp_off + udp_len + payload.len()],
        payload,
        reply_ip.src,
        reply_ip.dst,
    )?;

    Ok(ReplyFrame { bytes })
}

/// A reusable, socket-like template for replying to a specific UDP flow.
///
/// Intended usage:
/// - Build once from a received packet (`UdpReplyTemplate::from_parsed_packet`).
/// - Reuse for subsequent replies that should go back along the same L2/L3/L4 route,
///   only changing the payload.
///
/// This is the network-level “固化能力” for the server-side ergonomics problem.
#[derive(Debug, Clone, Copy)]
pub struct UdpReplyTemplate {
    pub eth: EthernetHeader,
    pub ip: Ipv4Header,
    pub udp: UdpHeader,
}

impl UdpReplyTemplate {
    /// Create a reply template by swapping src/dst of Ether/IPv4/UDP.
    ///
    /// `src_mac` is the MAC to use for the reply's Ethernet source.
    /// For multi-identity servers, this should be the chosen identity's MAC.
    pub fn from_layers(eth: &Ether, ip: &Ipv4, udp: &Udp, src_mac: MacAddr) -> Self {
        Self {
            eth: EthernetHeader {
                dst: eth.src,
                src: src_mac,
                ethertype: ETH_TYPE_IPV4,
            },
            ip: Ipv4Header {
                src: ip.dst,
                dst: ip.src,
                protocol: 17,
                ttl: ip.ttl,
                identification: ip.identification,
                flags_fragment: ip.flags_fragment,
            },
            udp: UdpHeader {
                src_port: udp.dst_port,
                dst_port: udp.src_port,
            },
        }
    }

    /// Create a reply template from a parsed packet.
    ///
    /// Errors if the packet does not contain Ether + Ipv4 + Udp.
    pub fn from_parsed_packet(pkt: &ParsedPacket<'_>, src_mac: MacAddr) -> anyhow::Result<Self> {
        let eth = pkt
            .get::<Ether>()
            .ok_or_else(|| anyhow::anyhow!("missing ether"))?;
        let ip = pkt
            .get::<Ipv4>()
            .ok_or_else(|| anyhow::anyhow!("missing ipv4"))?;
        let udp = pkt
            .get::<Udp>()
            .ok_or_else(|| anyhow::anyhow!("missing udp"))?;
        Ok(Self::from_layers(eth, ip, udp, src_mac))
    }

    /// Build a reply frame for `payload`.
    ///
    /// This always computes IPv4 header checksum and UDP checksum.
    pub fn build(self, payload: &[u8]) -> anyhow::Result<ReplyFrame> {
        let eth_len = EthernetHeader::LEN;
        let ip_len = Ipv4Header::MIN_LEN;
        let udp_len = UdpHeader::LEN;

        let mut bytes = vec![0u8; eth_len + ip_len + udp_len + payload.len()];

        self.eth.encode(&mut bytes[..eth_len])?;
        self.ip.encode(
            &mut bytes[eth_len..eth_len + ip_len],
            udp_len + payload.len(),
            0,
        )?;

        let udp_off = eth_len + ip_len;
        self.udp.encode(
            &mut bytes[udp_off..udp_off + udp_len + payload.len()],
            payload,
            self.ip.src,
            self.ip.dst,
        )?;

        Ok(ReplyFrame { bytes })
    }
}

/// A key that identifies a UDP flow (sufficient for echo-style servers).
///
/// This is purposely small and hashable so servers can keep a map of active flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UdpFlowKey {
    pub peer_ip: crate::Ipv4Addr,
    pub peer_port: u16,
    pub local_ip: crate::Ipv4Addr,
    pub local_port: u16,
}

impl UdpFlowKey {
    pub fn from_parsed_packet(pkt: &ParsedPacket<'_>) -> anyhow::Result<Self> {
        let ip = pkt
            .get::<Ipv4>()
            .ok_or_else(|| anyhow::anyhow!("missing ipv4"))?;
        let udp = pkt
            .get::<Udp>()
            .ok_or_else(|| anyhow::anyhow!("missing udp"))?;

        Ok(Self {
            peer_ip: ip.src,
            peer_port: udp.src_port,
            local_ip: ip.dst,
            local_port: udp.dst_port,
        })
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
