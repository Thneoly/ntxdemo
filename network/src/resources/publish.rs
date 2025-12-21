use crate::abr;
use crate::abr::BindingOwner;

use super::{NamedPools, PortPool, ResourcePools};

impl ResourcePools {
    /// Convenience: publish ABR view by adding bindings for pinned resources of an owner.
    ///
    /// This is deliberately a small helper for examples; higher-level components may want
    /// to reconcile bindings from multiple sources.
    pub fn publish_abr_for_owner(
        &self,
        store: &mut abr::BindingStore,
        owner: &super::OwnerId,
        abr_owner: BindingOwner,
    ) {
        store.clear();

        // IPv4 pinned allocations.
        for pool in self.ipv4.inner.values() {
            if let Some(ip) = pool.owner_to_pinned.get(owner).copied() {
                let ip_be = u32::from_be_bytes(ip.octets());
                store.add(abr::Binding::ipv4_be(ip_be, abr_owner));
            }
        }

        // Port bindings pinned allocations.
        //
        // If the owner has a pinned IPv4, publish (ip,port) bindings.
        // Otherwise, publish wildcard bindings using ip=0.0.0.0.
        //
        // Note: ABR has a WILDCARD flag, but `BindingStore` currently stores only keys.
        // Using ip=0.0.0.0 is enough for accept() paths that treat it as wildcard.
        let pinned_ip = self
            .ipv4
            .inner
            .values()
            .find_map(|p| p.owner_to_pinned.get(owner).copied());

        let ip_be = pinned_ip
            .map(|ip| u32::from_be_bytes(ip.octets()))
            .unwrap_or(0);

        publish_ports(
            store,
            &self.udp_port,
            owner,
            ip_be,
            abr_owner,
            |ip, port, o| abr::Binding::udp_port_be(ip, port, o),
        );
        publish_ports(
            store,
            &self.tcp_port,
            owner,
            ip_be,
            abr_owner,
            |ip, port, o| abr::Binding::tcp_port_be(ip, port, o),
        );

        abr::store_view(store.snapshot());
    }
}

fn publish_ports(
    store: &mut abr::BindingStore,
    pools: &NamedPools<PortPool>,
    owner: &super::OwnerId,
    ip_be: u32,
    abr_owner: BindingOwner,
    mk: fn(u32, u16, BindingOwner) -> abr::Binding,
) {
    for pool in pools.inner.values() {
        if let Some(ports) = pool.owner_to_pinned.get(owner) {
            for &port in ports {
                store.add(mk(ip_be, port, abr_owner));
            }
        }
    }
}
