use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use uuid::Uuid;

/// Globally unique id for any resource (socket / mac / ip / port ...).
///
/// This is a UUID v4.
pub type ResourceId = Uuid;

/// A process-local monotonically increasing id for flow keys (demux).
///
/// This complements `ResourceId`:
/// - `ResourceId` identifies a *socket instance* (ownership / control-plane)
/// - `sock_id` identifies a *flow key entry* (data-plane demux)
pub type SockId = u64;

static NEXT_SOCK_ID: AtomicU64 = AtomicU64::new(1);

#[inline]
pub fn alloc_sock_id() -> SockId {
    NEXT_SOCK_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Socket,
    Mac,
    Ipv4,
    UdpPort,
    TcpPort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketInfo {
    pub name: String,
    pub sock_id: Option<SockId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonSocketOwner {
    /// The socket resource id that owns this resource (used as `resources::OwnerId`).
    pub owner_id: ResourceId,

    /// The flow/socket-table id that is using this particular resource.
    ///
    /// Default: `Some(owner_sock_id)` when available at registration time.
    pub using_sock_id: Option<SockId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceRecord {
    Socket {
        info: SocketInfo,
    },
    NonSocket {
        kind: ResourceKind,
        owner: NonSocketOwner,
    },
}

/// In-memory registry for resource id relationships.
///
/// Notes:
/// - This is intentionally a pure data structure; locking is owned by the caller.
/// - It complements ABR: ABR is accept/filter for dataplane, registry is control-plane metadata.
#[derive(Debug, Default, Clone)]
pub struct ResourceRegistry {
    by_id: BTreeMap<ResourceId, ResourceRecord>,

    /// Fast reverse lookup: ipv4 -> owning socket owner id.
    ///
    /// Note: we keep this separate from `by_id` because `by_id` is keyed by ResourceId,
    /// while the kernel often observes packets by destination IP.
    ipv4_owner: BTreeMap<std::net::Ipv4Addr, ResourceId>,

    /// Owner socket -> resources owned by it.
    resources_by_owner: BTreeMap<ResourceId, BTreeSet<ResourceId>>,
}

impl ResourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn alloc_resource_id(&self) -> ResourceId {
        Uuid::new_v4()
    }

    pub fn register_socket(&mut self, socket_id: ResourceId, info: SocketInfo) {
        self.by_id
            .insert(socket_id, ResourceRecord::Socket { info });
        self.resources_by_owner.entry(socket_id).or_default();
    }

    /// Record a reverse mapping from an IPv4 address to its owning socket.
    pub fn register_ipv4_owner(&mut self, ipv4: std::net::Ipv4Addr, owner_id: ResourceId) {
        self.ipv4_owner.insert(ipv4, owner_id);
    }

    /// Reverse lookup: resolve an IPv4 address to its owning socket id.
    pub fn owner_of_ipv4(&self, ipv4: &std::net::Ipv4Addr) -> Option<ResourceId> {
        self.ipv4_owner.get(ipv4).copied()
    }

    pub fn register_non_socket(
        &mut self,
        resource_id: ResourceId,
        kind: ResourceKind,
        owner_id: ResourceId,
        using_sock_id: Option<SockId>,
    ) {
        self.by_id.insert(
            resource_id,
            ResourceRecord::NonSocket {
                kind,
                owner: NonSocketOwner {
                    owner_id,
                    using_sock_id,
                },
            },
        );
        self.resources_by_owner
            .entry(owner_id)
            .or_default()
            .insert(resource_id);
    }

    pub fn get(&self, id: &ResourceId) -> Option<&ResourceRecord> {
        self.by_id.get(id)
    }

    pub fn socket_info(&self, socket_id: &ResourceId) -> Option<&SocketInfo> {
        match self.by_id.get(socket_id) {
            Some(ResourceRecord::Socket { info }) => Some(info),
            _ => None,
        }
    }

    pub fn set_socket_sock_id(&mut self, socket_id: &ResourceId, sock_id: SockId) -> Option<()> {
        match self.by_id.get_mut(socket_id) {
            Some(ResourceRecord::Socket { info }) => {
                info.sock_id = Some(sock_id);
                Some(())
            }
            _ => None,
        }
    }

    pub fn kind_of(&self, id: &ResourceId) -> Option<ResourceKind> {
        match self.by_id.get(id) {
            Some(ResourceRecord::Socket { .. }) => Some(ResourceKind::Socket),
            Some(ResourceRecord::NonSocket { kind, .. }) => Some(*kind),
            None => None,
        }
    }

    pub fn owner_of(&self, id: &ResourceId) -> Option<ResourceId> {
        match self.by_id.get(id) {
            Some(ResourceRecord::NonSocket { owner, .. }) => Some(owner.owner_id),
            _ => None,
        }
    }

    pub fn using_sock_id_of(&self, id: &ResourceId) -> Option<SockId> {
        match self.by_id.get(id) {
            Some(ResourceRecord::NonSocket { owner, .. }) => owner.using_sock_id,
            _ => None,
        }
    }

    pub fn resources_of_owner(&self, owner_id: &ResourceId) -> Vec<ResourceId> {
        self.resources_by_owner
            .get(owner_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_reverse_lookup_works() {
        let mut reg = ResourceRegistry::new();

        let socket_id = Uuid::new_v4();
        reg.register_socket(
            socket_id,
            SocketInfo {
                name: "s1".to_string(),
                sock_id: None,
            },
        );

        let rid = Uuid::new_v4();
        reg.register_non_socket(rid, ResourceKind::Ipv4, socket_id, Some(10));

        assert_eq!(reg.kind_of(&rid), Some(ResourceKind::Ipv4));
        assert_eq!(reg.owner_of(&rid), Some(socket_id));
        assert_eq!(reg.using_sock_id_of(&rid), Some(10));
        assert_eq!(reg.socket_info(&socket_id).unwrap().name, "s1");

        let owned = reg.resources_of_owner(&socket_id);
        assert_eq!(owned, vec![rid]);
    }
}
