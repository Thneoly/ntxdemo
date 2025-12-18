//! Scapy-like packet layering DSL.
//!
//! Goal: allow ergonomic packet construction like:
//!
//! ```ignore
//! use ntx::network::stack::{layers, Raw, PacketBuilder};
//!
//! let pkt = layers::Ether { /* ... */ }
//!     / layers::Ipv4 { /* ... */ }
//!     / layers::Tcp { /* ... */ }
//!     / Raw::new(b"hello");
//! ```
//!
//! The result (`PacketBuilder`) can be turned into bytes via `build(&registry)`.

use std::ops::Div;

use super::{LayerInstance, LayerRegistry, layers, li};

/// Raw (opaque) payload bytes (like Scapy's `Raw(load=...)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raw(pub Vec<u8>);

impl Raw {
    #[inline]
    pub fn new(data: impl AsRef<[u8]>) -> Self {
        Self(data.as_ref().to_vec())
    }
}

/// A packet under construction: ordered layers (outer -> inner) + optional payload.
///
/// Contract:
/// - `layers` are in natural order: Ether, Ipv4, Tcp, ...
/// - `payload` (if any) is the innermost bytes.
#[derive(Debug, Default)]
pub struct PacketBuilder {
    pub layers: Vec<LayerInstance>,
    pub payload: Option<Vec<u8>>,
}

impl PacketBuilder {
    #[inline]
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            payload: None,
        }
    }

    /// Finalize into bytes.
    ///
    /// Uses `build_packet_with_glue` so UDP checksum glue still works.
    pub fn build(&self, registry: &LayerRegistry) -> Result<Vec<u8>, String> {
        super::build_packet_with_glue(
            &self.layers,
            self.payload.as_deref().unwrap_or(&[]),
            registry,
        )
    }

    /// Append a built-in Ether layer.
    #[must_use]
    pub fn ether(mut self, layer: layers::Ether) -> Self {
        self.layers.push(li::ether(layer));
        self
    }

    /// Append a built-in ARP layer.
    #[must_use]
    pub fn arp(mut self, layer: layers::Arp) -> Self {
        self.layers.push(li::arp(layer));
        self
    }

    /// Append a built-in IPv4 layer.
    #[must_use]
    pub fn ipv4(mut self, layer: layers::Ipv4) -> Self {
        self.layers.push(li::ipv4(layer));
        self
    }

    /// Append a built-in UDP layer.
    #[must_use]
    pub fn udp(mut self, layer: layers::Udp) -> Self {
        self.layers.push(li::udp(layer));
        self
    }

    /// Append a built-in TCP layer.
    #[must_use]
    pub fn tcp(mut self, layer: layers::Tcp) -> Self {
        self.layers.push(li::tcp(layer));
        self
    }

    /// Append a built-in VXLAN layer.
    #[must_use]
    pub fn vxlan(mut self, layer: layers::Vxlan) -> Self {
        self.layers.push(li::vxlan(layer));
        self
    }

    /// Set the innermost payload bytes (overwrites any previous payload).
    #[must_use]
    pub fn payload(mut self, payload: impl AsRef<[u8]>) -> Self {
        self.payload = Some(payload.as_ref().to_vec());
        self
    }

    /// Set the innermost payload bytes (overwrites any previous payload).
    #[must_use]
    pub fn raw(self, raw: Raw) -> Self {
        self.payload(raw.0)
    }
}

// --- Method-chaining starters ---------------------------------------------

/// Start a `PacketBuilder` chain from a layer.
///
/// This enables:
///
/// ```ignore
/// let pkt = layers::Ether{..}.chain().ipv4(layers::Ipv4{..}).udp(layers::Udp{..}).payload(b"hi");
/// ```
pub trait Chain {
    fn chain(self) -> PacketBuilder;
}

impl Chain for layers::Ether {
    fn chain(self) -> PacketBuilder {
        PacketBuilder::new().ether(self)
    }
}

impl Chain for layers::Arp {
    fn chain(self) -> PacketBuilder {
        PacketBuilder::new().arp(self)
    }
}

impl Chain for layers::Ipv4 {
    fn chain(self) -> PacketBuilder {
        PacketBuilder::new().ipv4(self)
    }
}

impl Chain for layers::Udp {
    fn chain(self) -> PacketBuilder {
        PacketBuilder::new().udp(self)
    }
}

impl Chain for layers::Tcp {
    fn chain(self) -> PacketBuilder {
        PacketBuilder::new().tcp(self)
    }
}

impl Chain for layers::Vxlan {
    fn chain(self) -> PacketBuilder {
        PacketBuilder::new().vxlan(self)
    }
}

/// Extension trait to start a packet builder chain from any built-in layer.
///
/// This exists because method chaining reads nicest when it starts with `.pkt()`:
///
/// ```ignore
/// let bytes = layers::Ether{..}
///     .pkt()
///     .ipv4(layers::Ipv4{..})
///     .udp(layers::Udp{..})
///     .payload(b"hi")
///     .build(&reg)?;
/// ```
///
/// `builder()` is provided as an alias for `pkt()`.
pub trait LayerPkt: Sized {
    /// Start building a packet with `self` as the first layer.
    fn pkt(self) -> PacketBuilder;

    /// Alias for [`pkt`](LayerPkt::pkt).
    fn builder(self) -> PacketBuilder {
        self.pkt()
    }
}

impl LayerPkt for layers::Ether {
    fn pkt(self) -> PacketBuilder {
        self.chain()
    }
}

impl LayerPkt for layers::Arp {
    fn pkt(self) -> PacketBuilder {
        self.chain()
    }
}

impl LayerPkt for layers::Ipv4 {
    fn pkt(self) -> PacketBuilder {
        self.chain()
    }
}

impl LayerPkt for layers::Udp {
    fn pkt(self) -> PacketBuilder {
        self.chain()
    }
}

impl LayerPkt for layers::Tcp {
    fn pkt(self) -> PacketBuilder {
        self.chain()
    }
}

impl LayerPkt for layers::Vxlan {
    fn pkt(self) -> PacketBuilder {
        self.chain()
    }
}

// --- Internal helper -------------------------------------------------------

#[inline]
fn push(mut b: PacketBuilder, layer: LayerInstance) -> PacketBuilder {
    b.layers.push(layer);
    b
}

// --- Start chain: Layer / Layer -> PacketBuilder ---------------------------

impl Div<layers::Ipv4> for layers::Ether {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Ipv4) -> Self::Output {
        let b = PacketBuilder {
            layers: vec![li::ether(self), li::ipv4(rhs)],
            payload: None,
        };
        b
    }
}

impl Div<layers::Udp> for layers::Ipv4 {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Udp) -> Self::Output {
        PacketBuilder {
            layers: vec![li::ipv4(self), li::udp(rhs)],
            payload: None,
        }
    }
}

impl Div<layers::Tcp> for layers::Ipv4 {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Tcp) -> Self::Output {
        PacketBuilder {
            layers: vec![li::ipv4(self), li::tcp(rhs)],
            payload: None,
        }
    }
}

impl Div<layers::Arp> for layers::Ether {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Arp) -> Self::Output {
        PacketBuilder {
            layers: vec![li::ether(self), li::arp(rhs)],
            payload: None,
        }
    }
}

impl Div<layers::Vxlan> for layers::Udp {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Vxlan) -> Self::Output {
        PacketBuilder {
            layers: vec![li::udp(self), li::vxlan(rhs)],
            payload: None,
        }
    }
}

impl Div<layers::Ether> for layers::Vxlan {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Ether) -> Self::Output {
        PacketBuilder {
            layers: vec![li::vxlan(self), li::ether(rhs)],
            payload: None,
        }
    }
}

// --- Continue chain: PacketBuilder / Layer --------------------------------

impl Div<layers::Ether> for PacketBuilder {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Ether) -> Self::Output {
        push(self, li::ether(rhs))
    }
}

impl Div<layers::Arp> for PacketBuilder {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Arp) -> Self::Output {
        push(self, li::arp(rhs))
    }
}

impl Div<layers::Ipv4> for PacketBuilder {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Ipv4) -> Self::Output {
        push(self, li::ipv4(rhs))
    }
}

impl Div<layers::Udp> for PacketBuilder {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Udp) -> Self::Output {
        push(self, li::udp(rhs))
    }
}

impl Div<layers::Tcp> for PacketBuilder {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Tcp) -> Self::Output {
        push(self, li::tcp(rhs))
    }
}

impl Div<layers::Vxlan> for PacketBuilder {
    type Output = PacketBuilder;
    fn div(self, rhs: layers::Vxlan) -> Self::Output {
        push(self, li::vxlan(rhs))
    }
}

// --- Payload terminators ----------------------------------------------------

impl Div<Raw> for PacketBuilder {
    type Output = PacketBuilder;
    fn div(mut self, rhs: Raw) -> Self::Output {
        self.payload = Some(rhs.0);
        self
    }
}

impl Div<Vec<u8>> for PacketBuilder {
    type Output = PacketBuilder;
    fn div(mut self, rhs: Vec<u8>) -> Self::Output {
        self.payload = Some(rhs);
        self
    }
}

impl<'a> Div<&'a [u8]> for PacketBuilder {
    type Output = PacketBuilder;
    fn div(mut self, rhs: &'a [u8]) -> Self::Output {
        self.payload = Some(rhs.to_vec());
        self
    }
}
