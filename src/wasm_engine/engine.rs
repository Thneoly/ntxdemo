use anyhow::Context;
use std::path::{Path, PathBuf};
use wasmtime::component::{Component, Func, Instance, Linker, types::ComponentItem};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::add_to_linker_sync};

use super::shared_mem;
use crate::event_bus::Bytes;
use ntx_network::resources::ResourcePools;
use ntx_network::socket::udp::UdpSocketBinder;
use ntx_network::socket::udp::create_udp_socket;
use parking_lot::Mutex;
use std::collections::HashMap;
use uuid::Uuid;

// Strongly-typed host bindings for our guest packet-engine component.
//
// This replaces string-based export lookup for the shared-memory dataplane ABI.
// Demo entrypoints (`handle-packet`, `run-scenario`) intentionally remain dynamic.
mod packet_engine_bindings {
    wasmtime::component::bindgen!({
        world: "ntx:packet/packet-engine",
        path: ["plugins/wit/host","plugins/wit/packet-engine"],
        debug:true,
    });
}

use packet_engine_bindings::ntx::hostnet::resources::{
    Host as ResourceHost, Ipv4Addr, MacAddr, ResourceError,
};
use packet_engine_bindings::ntx::hostnet::udp_socket_control::{
    FrameHandle, Host as UdpHost, SocketError, UdpSocket,
};
/// Host-managed TX frame arena (region=2).
///
/// This is a deliberately simple bump allocator for MVP:
/// - Fast write path
/// - Returns (region=2, offset, len)
/// - No free/reuse; wraps to 0 if out of space
#[derive(Debug)]
struct TxFrameArena {
    buf: Vec<u8>,
    head: usize,
}

impl TxFrameArena {
    const REGION: u32 = 2;

    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0u8; capacity],
            head: 0,
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Option<(u32, u32, u32)> {
        if bytes.len() > self.buf.len() {
            return None;
        }
        if self.head + bytes.len() > self.buf.len() {
            // Wrap to 0 (MVP). In the future we can make this a ring with fencing.
            self.head = 0;
        }
        let off = self.head;
        self.buf[off..off + bytes.len()].copy_from_slice(bytes);
        self.head += bytes.len();
        Some((Self::REGION, off as u32, bytes.len() as u32))
    }
}

#[derive(Debug)]
struct HostNetState {
    binder: UdpSocketBinder,
    /// Track sock_id -> owning socket ResourceId.
    sock_owner: HashMap<u64, Uuid>,
    tx_arena: TxFrameArena,
}

impl HostNetState {
    fn new() -> Self {
        // 1 MiB MVP arena.
        Self {
            binder: UdpSocketBinder::new(),
            sock_owner: HashMap::new(),
            tx_arena: TxFrameArena::new(1024 * 1024),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub component_path: PathBuf,
    /// Candidate function names to try at the component top-level.
    ///
    /// For the first demo we keep this generic. Later we can switch to typed bindgen.
    pub entry_candidates: Vec<String>,
}

impl EngineConfig {
    pub fn demo_default(component_path: impl Into<PathBuf>) -> Self {
        Self {
            component_path: component_path.into(),
            entry_candidates: vec!["handle-packet".into(), "run-scenario".into()],
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum EngineError {
    #[error("wasm engine init error: {0}")]
    Init(anyhow::Error),

    #[error("component entry function not found; tried {candidates:?}")]
    EntryNotFound { candidates: Vec<String> },

    #[error("component call failed: {0}")]
    Call(anyhow::Error),
}

impl From<anyhow::Error> for EngineError {
    fn from(value: anyhow::Error) -> Self {
        Self::Call(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxPacket {
    pub sock_id: Option<u64>,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineResult {
    pub did_work: bool,
    pub tx: Vec<TxPacket>,
}

pub struct State {
    wasi: WasiCtx,
    table: wasmtime::component::ResourceTable,

    /// Host-controlled network control-plane state for guest imports.
    pools: Mutex<ResourcePools>,
    udp_table: Mutex<ntx_network::socket::udp::Table>,
    hostnet: Mutex<HostNetState>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn parse_uuid(s: &str) -> Result<Uuid, String> {
    Uuid::parse_str(s).map_err(|e| format!("invalid uuid `{s}`: {e}"))
}

fn fmt_uuid(id: &Uuid) -> String {
    id.to_string()
}

impl ResourceHost for State {
    fn create_socket_owner(&mut self, name: String) -> wasmtime::Result<String, ResourceError> {
        let mut pools = self.pools.lock();
        let owner = pools.alloc_socket_owner(name);
        Ok(fmt_uuid(&owner))
    }

    fn alloc_ipv4(
        &mut self,
        pool: String,
        owner: String,
    ) -> wasmtime::Result<String, ResourceError> {
        let owner = parse_uuid(&owner).map_err(|e| ResourceError::Other(e))?;
        let mut pools = self.pools.lock();
        let (rid, _ip) = pools
            .alloc_ipv4_for_socket(&pool, owner)
            .map_err(|e| ResourceError::Other(e.to_string()))?;
        Ok(fmt_uuid(&rid))
    }

    fn alloc_mac(
        &mut self,
        pool: String,
        owner: String,
    ) -> wasmtime::Result<String, ResourceError> {
        let owner = parse_uuid(&owner).map_err(|e| ResourceError::Other(e))?;
        let mut pools = self.pools.lock();
        let (rid, _mac) = pools
            .alloc_mac_for_socket(&pool, owner)
            .map_err(|e| ResourceError::Other(e.to_string()))?;
        Ok(fmt_uuid(&rid))
    }

    fn alloc_udp_port(
        &mut self,
        pool: String,
        owner: String,
    ) -> wasmtime::Result<String, ResourceError> {
        let owner = parse_uuid(&owner).map_err(|e| ResourceError::Other(e))?;
        let mut pools = self.pools.lock();
        let (rid, _port) = pools
            .alloc_udp_port_for_socket(&pool, owner)
            .map_err(|e| ResourceError::Other(e.to_string()))?;
        Ok(fmt_uuid(&rid))
    }

    fn resolve_ipv4(&mut self, rid: String) -> wasmtime::Result<Ipv4Addr, ResourceError> {
        let rid = parse_uuid(&rid).map_err(ResourceError::Other)?;
        let pools = self.pools.lock();
        let ip = pools.resolve_ipv4(&rid).ok_or(ResourceError::NotFound)?;
        Ok(Ipv4Addr {
            a: ip.octets()[0],
            b: ip.octets()[1],
            c: ip.octets()[2],
            d: ip.octets()[3],
        })
    }

    fn resolve_mac(&mut self, rid: String) -> wasmtime::Result<MacAddr, ResourceError> {
        let rid = parse_uuid(&rid).map_err(ResourceError::Other)?;
        let pools = self.pools.lock();
        let mac = pools.resolve_mac(&rid).ok_or(ResourceError::NotFound)?;
        Ok(MacAddr {
            a: mac.0[0],
            b: mac.0[1],
            c: mac.0[2],
            d: mac.0[3],
            e: mac.0[4],
            f: mac.0[5],
        })
    }

    fn resolve_udp_port(&mut self, rid: String) -> wasmtime::Result<u16, ResourceError> {
        let rid = parse_uuid(&rid).map_err(ResourceError::Other)?;
        let pools = self.pools.lock();
        let port = pools
            .resolve_udp_port(&rid)
            .ok_or(ResourceError::NotFound)?;
        Ok(port)
    }
}

impl UdpHost for State {
    fn create(&mut self, name: String) -> wasmtime::Result<UdpSocket, SocketError> {
        let mut pools = self.pools.lock();
        let table = self.udp_table.lock();
        let (owner, sock) = create_udp_socket(&mut pools, &table, name);
        drop(table);

        self.hostnet.lock().sock_owner.insert(sock, owner);
        Ok(UdpSocket {
            owner: fmt_uuid(&owner),
            sock,
        })
    }

    fn bind_local_ipv4(
        &mut self,
        sock: u64,
        local_ipv4: String,
    ) -> wasmtime::Result<(), SocketError> {
        let rid = parse_uuid(&local_ipv4).map_err(SocketError::Other)?;
        self.hostnet.lock().binder.bind_local_ipv4_rid(sock, rid);
        Ok(())
    }

    fn bind_local_mac(
        &mut self,
        sock: u64,
        local_mac: String,
    ) -> wasmtime::Result<(), SocketError> {
        let rid = parse_uuid(&local_mac).map_err(SocketError::Other)?;
        self.hostnet.lock().binder.bind_local_mac_rid(sock, rid);
        Ok(())
    }

    fn bind_local_udp_port(
        &mut self,
        sock: u64,
        local_udp_port: String,
    ) -> wasmtime::Result<(), SocketError> {
        let rid = parse_uuid(&local_udp_port).map_err(SocketError::Other)?;
        self.hostnet
            .lock()
            .binder
            .bind_local_udp_port_rid(sock, rid);
        Ok(())
    }

    fn bind_peer(
        &mut self,
        sock: u64,
        peer_ipv4: packet_engine_bindings::ntx::hostnet::udp_socket_control::Ipv4Addr,
        peer_port: u16,
        peer_mac: packet_engine_bindings::ntx::hostnet::udp_socket_control::MacAddr,
    ) -> wasmtime::Result<(), packet_engine_bindings::ntx::hostnet::udp_socket_control::SocketError>
    {
        self.hostnet.lock().binder.bind_peer(
            sock,
            ntx_network::Ipv4Addr([peer_ipv4.a, peer_ipv4.b, peer_ipv4.c, peer_ipv4.d]),
            peer_port,
            ntx_network::MacAddr([
                peer_mac.a, peer_mac.b, peer_mac.c, peer_mac.d, peer_mac.e, peer_mac.f,
            ]),
        );
        Ok(())
    }

    fn bind_ttl(&mut self, sock: u64, ttl: u8) -> wasmtime::Result<(), SocketError> {
        self.hostnet.lock().binder.bind_ttl(sock, ttl);
        Ok(())
    }

    fn finalize(&mut self, sock: u64) -> wasmtime::Result<(), SocketError> {
        let pools = self.pools.lock();
        let mut table = self.udp_table.lock();
        self.hostnet
            .lock()
            .binder
            .finalize_into_table(&pools, &mut table, sock)
            .map_err(|e| SocketError::Other(e.to_string()))?;
        Ok(())
    }

    fn build_reply(
        &mut self,
        sock: u64,
        payload: Vec<u8>,
    ) -> wasmtime::Result<FrameHandle, SocketError> {
        let mut table = self.udp_table.lock();
        let frame = table
            .build_reply_for_sock_id(sock, &payload)
            .map_err(|e| SocketError::Other(e.to_string()))?;

        let mut hostnet = self.hostnet.lock();
        let Some((region, offset, len)) = hostnet.tx_arena.write(&frame.bytes) else {
            return Err(SocketError::NoSpace);
        };

        Ok(FrameHandle {
            region,
            offset,
            len,
        })
    }
}

pub struct ComponentEngine {
    cfg: EngineConfig,
    engine: Engine,
    store: Store<State>,
    instance: Instance,

    packet: Option<packet_engine_bindings::PacketEngine>,

    // Cached exports for the packet-engine ABI (optional).
    desc_get: Option<Func>,
    desc_put: Option<Func>,
    payload_put: Option<Func>,
    notify_rx: Option<Func>,

    // Shared-memory state (v1 single-memory layout, best-effort).
    shm_initialized: bool,
    next_seq: u64,
}

impl ComponentEngine {
    pub fn new(cfg: EngineConfig) -> Result<Self, EngineError> {
        let mut wasm_cfg = Config::new();
        wasm_cfg.wasm_component_model(true);
        wasm_cfg.async_support(false);

        let engine =
            Engine::new(&wasm_cfg).map_err(|e| EngineError::Init(anyhow::Error::from(e)))?;

        let mut store = Store::new(
            &engine,
            State {
                wasi: WasiCtxBuilder::new()
                    .inherit_stdio()
                    .inherit_network()
                    .build(),
                table: wasmtime::component::ResourceTable::default(),
                pools: Mutex::new(ResourcePools::empty()),
                udp_table: Mutex::new(ntx_network::socket::udp::Table::new(
                    ntx_network::ConnTableConfig {
                        max_entries: 4096,
                        ttl: None,
                    },
                )),
                hostnet: Mutex::new(HostNetState::new()),
            },
        );

        let mut linker: Linker<State> = Linker::new(&engine);
        add_to_linker_sync(&mut linker).map_err(|e| EngineError::Init(anyhow::Error::from(e)))?;

        // Wire the host WIT imports (`ntx:hostnet/*`) into the component linker.
        //
        // IMPORTANT: This uses the exact bindgen-generated signature for Wasmtime 39:
        //   PacketEngine::add_to_linker::<T, D>(linker, host_getter)
        // where `D` is a type that implements `HostWithStore` for the imported interfaces
        // and `host_getter` returns `D::Data<'_>`.
        packet_engine_bindings::PacketEngine::add_to_linker::<
            State,
            wasmtime::component::HasSelf<State>,
        >(&mut linker, |s| s)
        .map_err(|e| EngineError::Init(anyhow::Error::from(e)))?;

        let component = Component::from_file(&engine, &cfg.component_path)
            .with_context(|| format!("load component: {}", cfg.component_path.display()))
            .map_err(EngineError::Init)?;

        let instance = linker
            .instantiate(&mut store, &component)
            .context("instantiate component")
            .map_err(EngineError::Init)?;

        // Best-effort caching of known exports. Not all components implement these.
        let mut tmp = Self {
            cfg,
            engine,
            store,
            instance,
            shm_initialized: false,
            next_seq: 1,
            packet: None,
            desc_get: None,
            desc_put: None,
            payload_put: None,
            notify_rx: None,
        };

        // Prefer typed bindings for the packet-engine ABI.
        tmp.packet = packet_engine_bindings::PacketEngine::new(&mut tmp.store, &tmp.instance).ok();
        // Keep legacy cache as fallback (useful when a component doesn't match our WIT).
        tmp.cache_packet_engine_exports();

        return Ok(tmp);
    }

    fn cache_packet_engine_exports(&mut self) {
        self.desc_get = self.find_export_func("desc-get").ok();
        self.desc_put = self.find_export_func("desc-put").ok();
        self.payload_put = self.find_export_func("payload-put").ok();
        self.notify_rx = self.find_export_func("notify-rx").ok();
    }

    #[cfg(any(test, feature = "wasm-engine-test-access"))]
    pub fn store_mut(&mut self) -> &mut Store<State> {
        &mut self.store
    }

    #[cfg(any(test, feature = "wasm-engine-test-access"))]
    pub fn packet_typed(&self) -> Option<&packet_engine_bindings::PacketEngine> {
        self.packet.as_ref()
    }

    /// Initialize shared-memory ABI structures inside guest memory.
    ///
    /// For now this is best-effort and only flips an internal flag.
    /// The next step is to locate the exported linear memory from the component
    /// (or add explicit guest accessor funcs) and write `ControlBlock`.
    pub fn ensure_shared_mem_initialized(&mut self) {
        if self.shm_initialized {
            return;
        }
        // Best-effort: initialize the control block in the guest's `desc` buffer.
        // The guest demo component stores these buffers internally.
        let _ = self.init_ring_if_needed();
        self.shm_initialized = true;
    }

    /// Encode a demo input for the guest handler.
    ///
    /// Until the guest implements the shared-memory ABI, we feed JSON as string
    /// through the existing demo entrypoint, but we already structure sock+payload.
    pub fn build_demo_input_json(&mut self, sock_id: Option<u64>, payload: &[u8]) -> Bytes {
        self.ensure_shared_mem_initialized();
        let _seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        shared_mem::demo_json(sock_id, payload)
    }

    /// Enqueue one RX packet into the guest's shared buffers.
    ///
    /// This uses the demo guest ABI of `desc-put`/`payload-put`.
    pub fn enqueue_rx(&mut self, sock_id: Option<u64>, payload: &[u8]) -> Result<(), EngineError> {
        self.ensure_shared_mem_initialized();
        self.init_ring_if_needed()?;

        // Read current control block to get tail pointers.
        let mut desc_mem = self.desc_get()?;
        let cb = shared_mem::decode_control(&desc_mem).ok_or_else(|| {
            EngineError::Call(anyhow::anyhow!("guest desc buffer missing control block"))
        })?;

        // Payload ring: for the demo we just append at payload_tail (mod capacity).
        // We keep it simple (no wrap handling beyond truncation) because this is
        // an integration stepping stone.
        let mut payload_tail = cb.payload_tail as usize;
        let payload_capacity = cb.payload_capacity as usize;
        if payload_capacity == 0 {
            return Err(EngineError::Call(anyhow::anyhow!(
                "guest payload_capacity is 0"
            )));
        }

        // If payload is too large for remaining space, wrap to 0.
        if payload_tail + payload.len() > payload_capacity {
            payload_tail = 0;
        }

        self.payload_put(payload_tail as u32, payload)?;

        // Descriptor ring.
        let desc_capacity = cb.desc_capacity as usize;
        if desc_capacity == 0 {
            return Err(EngineError::Call(anyhow::anyhow!(
                "guest desc_capacity is 0"
            )));
        }

        let desc_tail = cb.desc_tail as usize;
        let slot = desc_tail % desc_capacity;
        let desc_off = shared_mem::DESCS_OFF as usize + slot * shared_mem::DESC_LEN;

        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let desc = shared_mem::Descriptor::rx(
            sock_id,
            (shared_mem::PAYLOAD_OFF as usize + payload_tail) as u32,
            payload.len() as u32,
            seq,
        );
        let enc_desc = shared_mem::encode_desc(&desc);
        // Ensure vec is large enough.
        if desc_mem.len() < desc_off + enc_desc.len() {
            desc_mem.resize(desc_off + enc_desc.len(), 0);
        }
        desc_mem[desc_off..desc_off + enc_desc.len()].copy_from_slice(&enc_desc);

        // Advance tail pointers.
        let mut new_cb = cb;
        new_cb.desc_tail = cb.desc_tail.wrapping_add(1);
        new_cb.payload_tail = (payload_tail + payload.len()) as u32;
        let enc_cb = shared_mem::encode_control(&new_cb);
        desc_mem[shared_mem::CONTROL_OFF as usize..shared_mem::CONTROL_OFF as usize + enc_cb.len()]
            .copy_from_slice(&enc_cb);

        // Write back descriptors/control.
        self.desc_put(0, &desc_mem)?;
        Ok(())
    }

    /// Smoke-test helper: calls the guest export `hostnet-smoke-create-owner`.
    ///
    /// This export may not exist in older component artifacts; in that case this
    /// function returns an error that callers can treat as "not supported".
    pub fn hostnet_smoke_create_owner(&mut self, name: &str) -> Result<String, EngineError> {
        // Dynamic fallback: try to call the export by name.
        let func = self.find_export_func("hostnet-smoke-create-owner")?;
        let typed = func
            .typed::<(&str,), (Result<String, String>,)>(&self.store)
            .context("hostnet-smoke-create-owner signature mismatch")
            .map_err(EngineError::Call)?;
        match typed
            .call(&mut self.store, (name,))
            .context("hostnet-smoke-create-owner call")
            .map_err(EngineError::Call)?
            .0
        {
            Ok(id) => Ok(id),
            Err(e) => Err(EngineError::Call(anyhow::anyhow!(e))),
        }
    }

    /// Call the guest `notify-rx` export.
    pub fn notify_rx(&mut self) -> Result<u32, EngineError> {
        if let Some(packet) = &self.packet {
            return packet
                .call_notify_rx(&mut self.store)
                .context("notify-rx call")
                .map_err(EngineError::Call);
        }

        // Fallback: dynamic lookup.
        let func = self
            .notify_rx
            .clone()
            .ok_or_else(|| EngineError::EntryNotFound {
                candidates: vec!["notify-rx".to_string()],
            })?;
        let typed = func
            .typed::<(), (u32,)>(&self.store)
            .context("notify-rx signature mismatch")
            .map_err(EngineError::Call)?;
        let (n,) = typed
            .call(&mut self.store, ())
            .context("notify-rx call")
            .map_err(EngineError::Call)?;
        Ok(n)
    }

    /// Convenience for tests: read current descriptor buffer.
    pub fn read_desc_buffer(&mut self) -> Result<Vec<u8>, EngineError> {
        self.desc_get()
    }

    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    /// Demo call wrapper.
    ///
    /// We support two styles for the first iteration:
    /// - `handle-packet(payload: string) -> result<string, string>` (interprets payload as utf8)
    /// - `run-scenario(yaml: string) -> result<string, string>` (same signature as `examples/call.rs`)
    pub fn call_demo(&mut self, input: &str) -> Result<EngineResult, EngineError> {
        let candidates = self.cfg.entry_candidates.clone();
        let func = self.find_top_level_func(&candidates)?;

        // Both demo candidates use the same function signature.
        let typed = func
            .typed::<(&str,), (Result<String, String>,)>(&self.store)
            .context("component function signature mismatch")
            .map_err(EngineError::Call)?;

        match typed
            .call(&mut self.store, (input,))
            .context("component call")
            .map_err(EngineError::Call)?
            .0
        {
            Ok(_summary) => Ok(EngineResult {
                did_work: true,
                tx: vec![],
            }),
            Err(e) => Err(EngineError::Call(anyhow::anyhow!(e))),
        }
    }

    fn find_top_level_func(&mut self, candidates: &[String]) -> Result<Func, EngineError> {
        for name in candidates {
            if let Some((item, idx)) = self.instance.get_export(&mut self.store, None, name) {
                if matches!(item, ComponentItem::ComponentFunc(_)) {
                    if let Some(f) = self.instance.get_func(&mut self.store, idx) {
                        return Ok(f);
                    }
                }
            }
        }
        Err(EngineError::EntryNotFound {
            candidates: candidates.to_vec(),
        })
    }

    fn find_export_func(&mut self, name: &str) -> Result<Func, EngineError> {
        if let Some((item, idx)) = self.instance.get_export(&mut self.store, None, name) {
            if matches!(item, ComponentItem::ComponentFunc(_)) {
                if let Some(f) = self.instance.get_func(&mut self.store, idx) {
                    return Ok(f);
                }
            }
        }
        Err(EngineError::EntryNotFound {
            candidates: vec![name.to_string()],
        })
    }

    fn init_ring_if_needed(&mut self) -> Result<(), EngineError> {
        // If control block already present, do nothing.
        if let Ok(desc) = self.desc_get() {
            if let Some(cb) = shared_mem::decode_control(&desc) {
                if cb.magic == shared_mem::NTX_MAGIC && cb.version == shared_mem::NTX_VERSION {
                    return Ok(());
                }
            }
        }

        // Pick small but non-trivial capacities for demo/testing.
        let cb = shared_mem::ControlBlock::new(64, 64 * 1024);
        let mut desc_mem = vec![
            0u8;
            shared_mem::DESCS_OFF as usize
                + (cb.desc_capacity as usize * shared_mem::DESC_LEN)
        ];
        let enc = shared_mem::encode_control(&cb);
        desc_mem[shared_mem::CONTROL_OFF as usize..shared_mem::CONTROL_OFF as usize + enc.len()]
            .copy_from_slice(&enc);
        self.desc_put(0, &desc_mem)?;
        Ok(())
    }

    fn desc_get(&mut self) -> Result<Vec<u8>, EngineError> {
        if let Some(packet) = &self.packet {
            return packet
                .call_desc_get(&mut self.store)
                .context("desc-get call")
                .map_err(EngineError::Call);
        }

        let func = self
            .desc_get
            .clone()
            .ok_or_else(|| EngineError::EntryNotFound {
                candidates: vec!["desc-get".to_string()],
            })?;
        let typed = func
            .typed::<(), (Vec<u8>,)>(&self.store)
            .context("desc-get signature mismatch")
            .map_err(EngineError::Call)?;
        let (buf,) = typed
            .call(&mut self.store, ())
            .context("desc-get call")
            .map_err(EngineError::Call)?;
        Ok(buf)
    }

    fn desc_put(&mut self, off: u32, data: &[u8]) -> Result<(), EngineError> {
        if let Some(packet) = &self.packet {
            return packet
                .call_desc_put(&mut self.store, off, data)
                .context("desc-put call")
                .map_err(EngineError::Call);
        }

        let func = self
            .desc_put
            .clone()
            .ok_or_else(|| EngineError::EntryNotFound {
                candidates: vec!["desc-put".to_string()],
            })?;
        let typed = func
            .typed::<(u32, Vec<u8>), ()>(&self.store)
            .context("desc-put signature mismatch")
            .map_err(EngineError::Call)?;
        typed
            .call(&mut self.store, (off, data.to_vec()))
            .context("desc-put call")
            .map_err(EngineError::Call)?;
        Ok(())
    }

    fn payload_put(&mut self, off: u32, data: &[u8]) -> Result<(), EngineError> {
        if let Some(packet) = &self.packet {
            return packet
                .call_payload_put(&mut self.store, off, data)
                .context("payload-put call")
                .map_err(EngineError::Call);
        }

        let func = self
            .payload_put
            .clone()
            .ok_or_else(|| EngineError::EntryNotFound {
                candidates: vec!["payload-put".to_string()],
            })?;
        let typed = func
            .typed::<(u32, Vec<u8>), ()>(&self.store)
            .context("payload-put signature mismatch")
            .map_err(EngineError::Call)?;
        typed
            .call(&mut self.store, (off, data.to_vec()))
            .context("payload-put call")
            .map_err(EngineError::Call)?;
        Ok(())
    }

    #[allow(dead_code)]
    fn component_path(&self) -> &Path {
        &self.cfg.component_path
    }
}
