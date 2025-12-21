use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;

use ntx_network::resources;

/// A lightweight, `src`-level audit/management registry.
///
/// Goals:
/// - Track what `request_resources()` allocated/pinned for which owner.
/// - Provide convenient queries for observability and debugging.
/// - Stay off the RX fast path (dataplane correlation remains conn-table based).
///
/// Non-goals (for now):
/// - Become the source of truth for allocation (that's `ResourcePools`).
/// - Drive dataplane demux.
#[derive(Debug, Default, Clone)]
pub struct AuditRegistry {
    owners: BTreeMap<resources::OwnerId, OwnerAudit>,
    ipv4_to_owner: BTreeMap<Ipv4Addr, resources::OwnerId>,
    resource_to_owner: BTreeMap<resources::ResourceId, resources::OwnerId>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OwnerAudit {
    pub name: Option<String>,
    pub ipv4: BTreeSet<(resources::ResourceId, Ipv4Addr)>,
    pub mac: BTreeSet<(resources::ResourceId, ntx_network::MacAddr)>,
    pub udp_ports: BTreeSet<(resources::ResourceId, u16)>,
    pub tcp_ports: BTreeSet<(resources::ResourceId, u16)>,
}

static AUDIT_REGISTRY: Lazy<Mutex<AuditRegistry>> =
    Lazy::new(|| Mutex::new(AuditRegistry::default()));

/// Mutate the global audit registry.
///
/// This keeps locking/ownership centralized and avoids leaking the Mutex type.
pub fn with_audit_registry_mut<R>(f: impl FnOnce(&mut AuditRegistry) -> R) -> R {
    let mut guard = AUDIT_REGISTRY.lock();
    f(&mut guard)
}

/// Read the global audit registry.
pub fn with_audit_registry<R>(f: impl FnOnce(&AuditRegistry) -> R) -> R {
    let guard = AUDIT_REGISTRY.lock();
    f(&guard)
}

impl AuditRegistry {
    pub fn record_owner_name(&mut self, owner: resources::OwnerId, name: Option<String>) {
        let entry = self.owners.entry(owner).or_default();
        if entry.name.is_none() {
            entry.name = name;
        }
    }

    pub fn record_ipv4(
        &mut self,
        owner: resources::OwnerId,
        rid: resources::ResourceId,
        ip: Ipv4Addr,
    ) {
        self.owners.entry(owner).or_default().ipv4.insert((rid, ip));
        self.ipv4_to_owner.insert(ip, owner);
        self.resource_to_owner.insert(rid, owner);
    }

    pub fn record_mac(
        &mut self,
        owner: resources::OwnerId,
        rid: resources::ResourceId,
        mac: ntx_network::MacAddr,
    ) {
        self.owners.entry(owner).or_default().mac.insert((rid, mac));
        self.resource_to_owner.insert(rid, owner);
    }

    pub fn record_udp_port(
        &mut self,
        owner: resources::OwnerId,
        rid: resources::ResourceId,
        port: u16,
    ) {
        self.owners
            .entry(owner)
            .or_default()
            .udp_ports
            .insert((rid, port));
        self.resource_to_owner.insert(rid, owner);
    }

    pub fn record_tcp_port(
        &mut self,
        owner: resources::OwnerId,
        rid: resources::ResourceId,
        port: u16,
    ) {
        self.owners
            .entry(owner)
            .or_default()
            .tcp_ports
            .insert((rid, port));
        self.resource_to_owner.insert(rid, owner);
    }

    pub fn owner_of_ipv4(&self, ip: &Ipv4Addr) -> Option<resources::OwnerId> {
        self.ipv4_to_owner.get(ip).copied()
    }

    pub fn owner_of_resource(&self, rid: &resources::ResourceId) -> Option<resources::OwnerId> {
        self.resource_to_owner.get(rid).copied()
    }

    pub fn owner_audit(&self, owner: &resources::OwnerId) -> Option<&OwnerAudit> {
        self.owners.get(owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn audit_registry_records_and_queries() {
        let owner = Uuid::new_v4();

        let ip_rid = Uuid::new_v4();
        let ip = Ipv4Addr::new(10, 0, 0, 2);

        let mac_rid = Uuid::new_v4();
        let mac = ntx_network::MacAddr([1, 2, 3, 4, 5, 6]);

        let udp_rid = Uuid::new_v4();
        let tcp_rid = Uuid::new_v4();

        let snapshot = with_audit_registry_mut(|reg| {
            reg.record_owner_name(owner, Some("s1".to_string()));
            reg.record_ipv4(owner, ip_rid, ip);
            reg.record_mac(owner, mac_rid, mac);
            reg.record_udp_port(owner, udp_rid, 1234);
            reg.record_tcp_port(owner, tcp_rid, 4321);

            reg.clone()
        });

        assert_eq!(snapshot.owner_of_ipv4(&ip), Some(owner));
        assert_eq!(snapshot.owner_of_resource(&ip_rid), Some(owner));
        assert_eq!(snapshot.owner_of_resource(&mac_rid), Some(owner));
        assert_eq!(snapshot.owner_of_resource(&udp_rid), Some(owner));
        assert_eq!(snapshot.owner_of_resource(&tcp_rid), Some(owner));

        let oa = snapshot.owner_audit(&owner).expect("owner audit exists");
        assert_eq!(oa.name.as_deref(), Some("s1"));
        assert!(oa.ipv4.contains(&(ip_rid, ip)));
        assert!(oa.mac.contains(&(mac_rid, mac)));
        assert!(oa.udp_ports.contains(&(udp_rid, 1234)));
        assert!(oa.tcp_ports.contains(&(tcp_rid, 4321)));
    }
}
