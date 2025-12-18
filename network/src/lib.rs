//! Standalone userspace networking crate.
pub mod abr;
mod fmt;
mod nic;
pub mod packet;
pub mod prelude;
pub mod stack;
pub mod traffic;

#[cfg(test)]
mod abr_tests;

// Back-compat: keep old top-level paths by re-exporting from `packet::headers`.
// (These are used by various NIC + stack code.)
pub use packet::headers::{
    ArpCache, ArpPacket, ETH_TYPE_ARP, ETH_TYPE_IPV4, EthernetHeader, Ipv4Addr, Ipv4Header,
    MacAddr, TcpFlags, TcpHeader, UdpHeader,
};

// Back-compat helpers.
pub use packet::headers::{ipv4_header_checksum, tcp_checksum, udp_checksum};

// Convenient re-exports (preserve the old API surface used by the main binary).
#[allow(unused_imports)]
pub use nic::{AfPacketDgramNic, AfPacketNic, Nic, TpacketV3Nic};
#[allow(unused_imports)]
pub use stack::{
    // Pipeline surface
    Action,
    // Core registry + layer types
    BindKey,
    EdgeKind,

    Layer,
    LayerId,
    LayerInstance,
    LayerRegistry,
    PacketGraph,
    PacketHandler,
    ParsedPacket,
    Pipeline,
    ReplyFrame,
    UdpEchoHandler,
    // Parser / graph
    build_packet,
    build_udp_reply,
    default_registry,

    parse_packet,
    parse_packet_graph,
    parse_packet_with_spans,
};

// Note: don't glob re-export `packet::*` to avoid ambiguous re-exports with `stack::*`.
