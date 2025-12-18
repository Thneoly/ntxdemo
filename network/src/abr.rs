//! Active Binding Resource (ABR)
//!
//! ABR is the dataplane "source of truth": the set of resources currently bound/owned
//! by this node and therefore should be accepted by the stack.
//!
//! Design constraints:
//! - Dataplane queries must be O(1) or O(log N).
//! - Fast path is lock-free (single atomic load).
//! - Hot updates are supported via snapshot swapping (RCU style).
//! - The read-only view is eBPF/WASM friendly (arrays/bitmaps, no pointers into
//!   mutable structures).
//!
//! Non-goals:
//! - ABR does not allocate, decide lifetimes, or do conntrack.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::LazyLock;

use arc_swap::ArcSwap;

/// Kind discriminator for bindings.
///
/// Kept `repr(u8)` so it can be shared with other runtimes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Ipv4Addr = 1,
    Ipv6Addr = 2,
    TcpPort = 3,
    UdpPort = 4,
    Vni = 5,
}

/// Query key for a binding.
///
/// Note: keep this small and copyable; it is control-plane only in this crate
/// (dataplane queries go through `ResourceView`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BindingKey {
    /// IPv4 address in big-endian u32 (network order).
    Ipv4(u32),
    TcpPort {
        ip: u32,
        port: u16,
    },
    UdpPort {
        ip: u32,
        port: u16,
    },
    Vni(u32),
}

/// Ownership metadata (non-fast-path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingOwner {
    KernelIface,
    Container { id: u64 },
    Process { pid: u32 },
    Tunnel { id: u32 },
}

bitflags::bitflags! {
    /// Optional binding flags (non-fast-path).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct BindingFlags: u32 {
        const NONE = 0;
        /// Binding is considered "wildcard" (e.g. 0.0.0.0:port).
        const WILDCARD = 1 << 0;
    }
}

/// A binding is a fact record: some layer occupies some resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Binding {
    pub kind: ResourceKind,
    pub key: BindingKey,
    pub owner: BindingOwner,
    pub flags: BindingFlags,
}

impl Binding {
    pub fn ipv4_be(ip: u32, owner: BindingOwner) -> Self {
        Self {
            kind: ResourceKind::Ipv4Addr,
            key: BindingKey::Ipv4(ip),
            owner,
            flags: BindingFlags::NONE,
        }
    }

    pub fn udp_port_be(ip: u32, port: u16, owner: BindingOwner) -> Self {
        Self {
            kind: ResourceKind::UdpPort,
            key: BindingKey::UdpPort { ip, port },
            owner,
            flags: BindingFlags::NONE,
        }
    }

    pub fn tcp_port_be(ip: u32, port: u16, owner: BindingOwner) -> Self {
        Self {
            kind: ResourceKind::TcpPort,
            key: BindingKey::TcpPort { ip, port },
            owner,
            flags: BindingFlags::NONE,
        }
    }
}

/// Fast-path IPv4 ownership set.
///
/// eBPF/WASM-friendly: sorted array => O(log N) via binary search.
#[derive(Debug, Clone, Default)]
pub struct Ipv4Set {
    addrs_be: Arc<[u32]>,
}

impl Ipv4Set {
    pub fn empty() -> Self {
        Self {
            addrs_be: Arc::from([]),
        }
    }

    #[inline]
    pub fn contains_be(&self, ip_be: u32) -> bool {
        self.addrs_be.binary_search(&ip_be).is_ok()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.addrs_be.is_empty()
    }
}

/// (ip, port) sorted set. Placeholder implementation using an array.
///
/// Future-proofing: can be replaced by a hash/bitmap/LPM without affecting callers.
#[derive(Debug, Clone, Default)]
pub struct IpPortSet {
    pairs_be: Arc<[(u32, u16)]>,
}

/// Fast-path VNI set for VXLAN.
///
/// Stored as sorted u32 so it can be projected to eBPF maps or WASM tables.
#[derive(Debug, Clone, Default)]
pub struct VniSet {
    vnis: Arc<[u32]>,
}

impl VniSet {
    pub fn empty() -> Self {
        Self {
            vnis: Arc::from([]),
        }
    }

    #[inline]
    pub fn contains(&self, vni: u32) -> bool {
        self.vnis.binary_search(&vni).is_ok()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.vnis.is_empty()
    }
}

impl IpPortSet {
    pub fn empty() -> Self {
        Self {
            pairs_be: Arc::from([]),
        }
    }

    #[inline]
    pub fn contains_be(&self, ip_be: u32, port: u16) -> bool {
        self.pairs_be.binary_search(&(ip_be, port)).is_ok()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pairs_be.is_empty()
    }
}

/// Read-only ABR snapshot. Dataplane only reads this.
#[derive(Debug, Clone, Default)]
pub struct ResourceView {
    pub ipv4: Ipv4Set,
    pub tcp_ports: IpPortSet,
    pub udp_ports: IpPortSet,
    pub vni: VniSet,
}

impl ResourceView {
    pub fn empty() -> Self {
        Self {
            ipv4: Ipv4Set::empty(),
            tcp_ports: IpPortSet::empty(),
            udp_ports: IpPortSet::empty(),
            vni: VniSet::empty(),
        }
    }
}

/// Mutable control-plane store.
///
/// This can be locked and updated at low frequency. The dataplane never touches it.
#[derive(Debug, Default, Clone)]
pub struct BindingStore {
    ipv4_be: BTreeSet<u32>,
    tcp_be: BTreeSet<(u32, u16)>,
    udp_be: BTreeSet<(u32, u16)>,
    vni: BTreeSet<u32>,
}

impl BindingStore {
    pub fn clear(&mut self) {
        self.ipv4_be.clear();
        self.tcp_be.clear();
        self.udp_be.clear();
        self.vni.clear();
    }

    pub fn add(&mut self, binding: Binding) {
        match binding.key {
            BindingKey::Ipv4(ip) => {
                self.ipv4_be.insert(ip);
            }
            BindingKey::TcpPort { ip, port } => {
                self.tcp_be.insert((ip, port));
            }
            BindingKey::UdpPort { ip, port } => {
                self.udp_be.insert((ip, port));
            }
            BindingKey::Vni(vni) => {
                self.vni.insert(vni);
            }
        }
    }

    pub fn remove(&mut self, binding: &Binding) {
        match binding.key {
            BindingKey::Ipv4(ip) => {
                self.ipv4_be.remove(&ip);
            }
            BindingKey::TcpPort { ip, port } => {
                self.tcp_be.remove(&(ip, port));
            }
            BindingKey::UdpPort { ip, port } => {
                self.udp_be.remove(&(ip, port));
            }
            BindingKey::Vni(vni) => {
                self.vni.remove(&vni);
            }
        }
    }

    /// Create an immutable snapshot for dataplane consumption.
    pub fn snapshot(&self) -> ResourceView {
        let ipv4: Vec<u32> = self.ipv4_be.iter().copied().collect();
        let tcp: Vec<(u32, u16)> = self.tcp_be.iter().copied().collect();
        let udp: Vec<(u32, u16)> = self.udp_be.iter().copied().collect();
        let vni: Vec<u32> = self.vni.iter().copied().collect();

        ResourceView {
            ipv4: Ipv4Set {
                addrs_be: Arc::from(ipv4.into_boxed_slice()),
            },
            tcp_ports: IpPortSet {
                pairs_be: Arc::from(tcp.into_boxed_slice()),
            },
            udp_ports: IpPortSet {
                pairs_be: Arc::from(udp.into_boxed_slice()),
            },
            vni: VniSet {
                vnis: Arc::from(vni.into_boxed_slice()),
            },
        }
    }
}

/// Global RCU-style pointer to the current ABR view.
///
/// Dataplane: `load()` is a single atomic op.
/// Control plane: `store()` swaps the snapshot in O(1).
static RESOURCE_VIEW: LazyLock<ArcSwap<ResourceView>> =
    LazyLock::new(|| ArcSwap::from_pointee(ResourceView::empty()));

/// Load current ABR snapshot.
#[inline]
pub fn load_view() -> Arc<ResourceView> {
    RESOURCE_VIEW.load_full()
}

/// Replace ABR snapshot (hot update).
#[inline]
pub fn store_view(view: ResourceView) {
    RESOURCE_VIEW.store(Arc::new(view));
}

/// A low-frequency facts provider for ABR.
///
/// Implementations should write their observed active bindings into `store`.
///
/// Notes:
/// - This is *not* a desired-state config interface.
/// - It should be safe to call periodically; `reconcile_once` clears the store first.
pub trait BindingProvider {
    fn reconcile(&self, store: &mut BindingStore);
}

/// Reconcile all providers into `store`, snapshot it into a read-only [`ResourceView`], and
/// publish it as the current dataplane ABR view.
///
/// This function intentionally does *not* loop or sleep; callers decide cadence.
///
/// Recommended usage pattern:
/// - Have a control-plane thread/task call this periodically.
/// - Dataplane loads a stable snapshot once per batch and carries it in `PacketContext`.
pub fn reconcile_once(providers: &[&dyn BindingProvider], store: &mut BindingStore) {
    store.clear();
    for p in providers {
        p.reconcile(store);
    }
    let view = store.snapshot();
    store_view(view);
}
