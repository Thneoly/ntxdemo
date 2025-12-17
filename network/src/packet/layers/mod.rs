//! Built-in `Layer` implementations.
//!
//! `stack` orchestrates parsing/building; these are the protocol implementations.

mod arp;
mod ether;
mod ipv4;
mod tcp;
mod udp;
mod vxlan;

pub use arp::Arp;
pub use ether::Ether;
pub use ipv4::Ipv4;
pub use tcp::Tcp;
pub use udp::Udp;
pub use vxlan::Vxlan;

use crate::stack::{BindKey, Layer, LayerId, LayerInstance, LayerRegistry};

/// Register all built-in protocol layers.
pub fn register_all(reg: &mut LayerRegistry) {
    // Default bindings (Scapy-like): UDP dport 4789 => VXLAN
    reg.bind(LayerId::Udp, BindKey::UdpDstPort(4789), LayerId::Vxlan);

    // Default tunnel semantics: VXLAN payload is an inner Ethernet frame.
    reg.register_tunnel(LayerId::Vxlan, |any, _payload| {
        // Validate type to avoid silent misuse.
        any.downcast_ref::<Vxlan>()
            .ok_or_else(|| "tunnel type mismatch: Vxlan".to_string())?;
        Ok(Some(LayerId::Ether))
    });

    // Ether
    reg.register_decoder(LayerId::Ether, |data| {
        let (l, n) = Ether::decode(data)?;
        Ok((
            LayerInstance {
                id: LayerId::Ether,
                inner: Box::new(l),
            },
            n,
            l.next_layer(),
            None,
        ))
    });
    reg.register_encoder(LayerId::Ether, |any, payload, out| {
        let l = any
            .downcast_ref::<Ether>()
            .ok_or_else(|| "encoder type mismatch: Ether".to_string())?;
        l.encode(payload, out);
        Ok(())
    });

    // ARP
    reg.register_decoder(LayerId::Arp, |data| {
        let (l, n) = Arp::decode(data)?;
        Ok((
            LayerInstance {
                id: LayerId::Arp,
                inner: Box::new(l),
            },
            n,
            l.next_layer(),
            None,
        ))
    });
    reg.register_encoder(LayerId::Arp, |any, payload, out| {
        let l = any
            .downcast_ref::<Arp>()
            .ok_or_else(|| "encoder type mismatch: Arp".to_string())?;
        l.encode(payload, out);
        Ok(())
    });

    // IPv4
    reg.register_decoder(LayerId::Ipv4, |data| {
        let (l, n) = Ipv4::decode(data)?;
        Ok((
            LayerInstance {
                id: LayerId::Ipv4,
                inner: Box::new(l),
            },
            n,
            l.next_layer(),
            None,
        ))
    });
    reg.register_encoder(LayerId::Ipv4, |any, payload, out| {
        let l = any
            .downcast_ref::<Ipv4>()
            .ok_or_else(|| "encoder type mismatch: Ipv4".to_string())?;
        l.encode(payload, out);
        Ok(())
    });

    // UDP
    reg.register_decoder(LayerId::Udp, |data| {
        let (l, n) = Udp::decode(data)?;
        let bind = Some(BindKey::UdpDstPort(l.dst_port));
        Ok((
            LayerInstance {
                id: LayerId::Udp,
                inner: Box::new(l),
            },
            n,
            l.next_layer(),
            bind,
        ))
    });
    reg.register_encoder(LayerId::Udp, |any, payload, out| {
        let l = any
            .downcast_ref::<Udp>()
            .ok_or_else(|| "encoder type mismatch: Udp".to_string())?;
        l.encode(payload, out);
        Ok(())
    });

    // TCP
    reg.register_decoder(LayerId::Tcp, |data| {
        let (l, n) = Tcp::decode(data)?;
        let bind = Some(BindKey::TcpDstPort(l.dst_port));
        Ok((
            LayerInstance {
                id: LayerId::Tcp,
                inner: Box::new(l),
            },
            n,
            l.next_layer(),
            bind,
        ))
    });
    reg.register_encoder(LayerId::Tcp, |any, payload, out| {
        let l = any
            .downcast_ref::<Tcp>()
            .ok_or_else(|| "encoder type mismatch: Tcp".to_string())?;
        l.encode(payload, out);
        Ok(())
    });

    // VXLAN
    reg.register_decoder(LayerId::Vxlan, |data| {
        let (l, n) = Vxlan::decode(data)?;
        Ok((
            LayerInstance {
                id: LayerId::Vxlan,
                inner: Box::new(l),
            },
            n,
            l.next_layer(),
            None,
        ))
    });
    reg.register_encoder(LayerId::Vxlan, |any, payload, out| {
        let l = any
            .downcast_ref::<Vxlan>()
            .ok_or_else(|| "encoder type mismatch: Vxlan".to_string())?;
        l.encode(payload, out);
        Ok(())
    });
}
