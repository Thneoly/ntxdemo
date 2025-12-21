//! Socket as a first-class resource.
//!
//! In this codebase we distinguish:
//! - `ResourceId` (`uuid::Uuid`): globally unique identifier used by control-plane / ownership.
//! - `SockId` (`u64`): process-local monotonically increasing id used for dataplane demux.
//!
//! This module focuses on the control-plane facing *socket resource* shape.

use serde::{Deserialize, Serialize};

use super::{ResourceId, SockId};

/// A socket resource (control-plane identity).
///
/// Typically this is also used as the `OwnerId` for resources allocated on behalf of that socket.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SocketResource {
    /// Stable id used by control-plane / ABR.
    pub id: ResourceId,

    /// Human-friendly name (optional).
    #[serde(default)]
    pub name: String,

    /// Optional dataplane sock id associated with this socket.
    ///
    /// Notes:
    /// - This may be `None` when the socket is created purely from config without any flow.
    /// - It may be set later once a concrete flow entry exists.
    #[serde(default)]
    pub sock_id: Option<SockId>,
}

impl SocketResource {
    pub fn new(id: ResourceId) -> Self {
        Self {
            id,
            name: String::new(),
            sock_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_resource_create_and_alloc_sock_id() {
        let id = alloc_socket_resource_id();
        let s = SocketResource::new(id);
        assert_eq!(s.id, id);
        assert!(s.name.is_empty());
        assert!(s.sock_id.is_none());

        // `SockId` is a process-local monotonic counter.
        let a = alloc_sock_id();
        let b = alloc_sock_id();
        assert!(b > a);
    }
}

/// Convenience helper for allocating a fresh socket resource id.
#[inline]
pub fn alloc_socket_resource_id() -> ResourceId {
    uuid::Uuid::new_v4()
}

// Re-export dataplane sock-id helper here so callers can opt-in to the semantic module.
#[allow(unused_imports)]
pub use super::id::alloc_sock_id;
