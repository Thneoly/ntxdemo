use anyhow::Context;
use std::path::PathBuf;
use wasmtime::component::{Component, Func, Instance, Linker, types::ComponentItem};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{
    DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::add_to_linker_sync,
};

use crate::kernel;
use crate::rx_ring::{RxRing, RxRingConfig};
use ntx_network;

// Strongly-typed host bindings for imports required by the composed scheduler component.
mod packet_engine_bindings {
    wasmtime::component::bindgen!({
        world: "ntx:host/hostnet",
        // This package has file-based deps under `plugins/wit/packet-engine/deps/*`.
        path: ["component/wit/host"],
        debug:true,
    });
}
use packet_engine_bindings::ntx::host::resources::{
    Host as ResourceHost, ResourceError, UdpIdentity,
};
use packet_engine_bindings::ntx::host::types::{Host as TypesHost, Ipv4Addr, MacAddr};
// (types imported via WIT bindings in signatures; concrete conversions use ntx_network types)
use packet_engine_bindings::ntx::host::rx_ring::{Host as RxRingHost, RxBatch as WitRxBatch};
use packet_engine_bindings::ntx::host::udp_socket_control::{
    FrameHandle, Host as UdpHost, SocketError, UdpBind, UdpSocket,
};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub component_path: PathBuf,
}

impl EngineConfig {
    pub fn composed_scheduler_default(component_path: impl Into<PathBuf>) -> Self {
        Self {
            component_path: component_path.into(),
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

pub struct State {
    wasi: WasiCtx,
    table: wasmtime::component::ResourceTable,
    rx_ring: RxRing,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl TypesHost for State {}
impl ResourceHost for State {
    fn create_socket_owner(&mut self, name: String) -> wasmtime::Result<String, ResourceError> {
        // Delegate to kernel control-plane (which wraps `ntx_network::resources`).
        // NOTE: kernel expects to own the pools; wasm_engine shouldn't maintain a parallel copy.
        kernel::hostnet_create_socket_owner(&name).map_err(|e| ResourceError::Other(e.to_string()))
    }
    fn acquire_udp_port(
        &mut self,
        pool: String,
        owner: String,
    ) -> wasmtime::Result<(), ResourceError> {
        kernel::hostnet_acquire_udp_port(&pool, &owner)
            .map_err(|e| ResourceError::Other(e.to_string()))?;
        Ok(())
    }

    fn acquire_udp_identity(
        &mut self,
        pool: String,
        owner: String,
    ) -> wasmtime::Result<UdpIdentity, ResourceError> {
        let (ip, mac, port) = kernel::hostnet_acquire_udp_identity(&pool, &owner)
            .map_err(|e| ResourceError::Other(e.to_string()))?;
        Ok(UdpIdentity {
            local_ipv4: Ipv4Addr {
                a: ip.octets()[0],
                b: ip.octets()[1],
                c: ip.octets()[2],
                d: ip.octets()[3],
            },
            local_mac: MacAddr {
                a: mac.0[0],
                b: mac.0[1],
                c: mac.0[2],
                d: mac.0[3],
                e: mac.0[4],
                f: mac.0[5],
            },
            local_udp_port: port,
        })
    }

    fn resolve_peer_mac(
        &mut self,
        peer_ipv4: Ipv4Addr,
    ) -> wasmtime::Result<MacAddr, ResourceError> {
        let mac = kernel::hostnet_resolve_peer_mac(ntx_network::Ipv4Addr([
            peer_ipv4.a,
            peer_ipv4.b,
            peer_ipv4.c,
            peer_ipv4.d,
        ]))
        .map_err(|e| match e {
            kernel::HostnetError::NotFound(_) => ResourceError::NotFound,
            kernel::HostnetError::InvalidArgument(_) => ResourceError::InvalidArgument,
            other => ResourceError::Other(other.to_string()),
        })?;
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
        kernel::hostnet_resolve_udp_port(&rid).map_err(|e| match e {
            kernel::HostnetError::NotFound(_) => ResourceError::NotFound,
            other => ResourceError::Other(other.to_string()),
        })
    }

    fn release_resource(&mut self, _rid: String) -> wasmtime::Result<(), ResourceError> {
        // Best-effort cleanup: the host control-plane owns pool lifecycle.
        // For now we keep this as a no-op to satisfy the WIT contract.
        Ok(())
    }
}

impl UdpHost for State {
    fn create(&mut self, name: String) -> wasmtime::Result<UdpSocket, SocketError> {
        let created =
            kernel::hostnet_udp_create(&name).map_err(|e| SocketError::Other(e.to_string()))?;

        Ok(UdpSocket {
            owner: created.owner,
            sock: created.sock_id,
        })
    }

    fn bind(&mut self, sock: u64, b: UdpBind) -> wasmtime::Result<(), SocketError> {
        kernel::hostnet_udp_bind_all(
            sock,
            ntx_network::Ipv4Addr([
                b.local_ipv4.a,
                b.local_ipv4.b,
                b.local_ipv4.c,
                b.local_ipv4.d,
            ]),
            ntx_network::MacAddr([
                b.local_mac.a,
                b.local_mac.b,
                b.local_mac.c,
                b.local_mac.d,
                b.local_mac.e,
                b.local_mac.f,
            ]),
            b.local_udp_port,
            ntx_network::Ipv4Addr([b.peer_ipv4.a, b.peer_ipv4.b, b.peer_ipv4.c, b.peer_ipv4.d]),
            b.peer_port,
            ntx_network::MacAddr([
                b.peer_mac.a,
                b.peer_mac.b,
                b.peer_mac.c,
                b.peer_mac.d,
                b.peer_mac.e,
                b.peer_mac.f,
            ]),
            b.ttl,
        )
        .map_err(|e| SocketError::Other(e.to_string()))
    }

    fn build_reply(
        &mut self,
        sock: u64,
        payload: Vec<u8>,
    ) -> wasmtime::Result<FrameHandle, SocketError> {
        let handle = kernel::hostnet_udp_build_reply(sock, &payload).map_err(|e| match e {
            kernel::HostnetError::NoSpace => SocketError::NoSpace,
            other => SocketError::Other(other.to_string()),
        })?;
        Ok(FrameHandle {
            region: handle.region,
            offset: handle.offset,
            len: handle.len,
        })
    }

    fn tx(&mut self, frame: FrameHandle) -> wasmtime::Result<u32, SocketError> {
        kernel::hostnet_tx(kernel::HostnetFrameHandle {
            region: frame.region,
            offset: frame.offset,
            len: frame.len,
        })
        .map_err(|e| match e {
            kernel::HostnetError::InvalidArgument(_) => SocketError::InvalidArgument,
            kernel::HostnetError::NotFound(_) => SocketError::NotFound,
            kernel::HostnetError::NoSpace => SocketError::NoSpace,
            other => SocketError::Other(other.to_string()),
        })
    }
}

impl RxRingHost for State {
    fn poll_rx(&mut self, max_desc: u32, max_payload: u32) -> Option<WitRxBatch> {
        self.rx_ring
            .poll_rx(max_desc, max_payload)
            .map(|b| WitRxBatch {
                handle: b.handle,
                desc_len: b.desc_len,
                payload_len: b.payload_len,
                seq: b.seq,
            })
    }

    fn wait_rx(&mut self, max_desc: u32, max_payload: u32, timeout_ms: u32) -> Option<WitRxBatch> {
        self.rx_ring
            .wait_rx(max_desc, max_payload, timeout_ms)
            .map(|b| WitRxBatch {
                handle: b.handle,
                desc_len: b.desc_len,
                payload_len: b.payload_len,
                seq: b.seq,
            })
    }

    fn read_desc(&mut self, handle: u64, off: u32, len: u32) -> Result<Vec<u8>, String> {
        self.rx_ring.read_desc(handle, off, len)
    }

    fn read_payload(&mut self, handle: u64, off: u32, len: u32) -> Result<Vec<u8>, String> {
        self.rx_ring.read_payload(handle, off, len)
    }

    fn release(&mut self, handle: u64) -> Result<(), String> {
        self.rx_ring.release(handle)
    }
}

pub struct ComponentEngine {
    cfg: EngineConfig,
    store: Store<State>,
    run: Func,
}

impl ComponentEngine {
    pub fn new(cfg: EngineConfig) -> Result<Self, EngineError> {
        let mut wasm_cfg = Config::new();
        wasm_cfg.wasm_component_model(true);
        wasm_cfg.async_support(false);

        let engine =
            Engine::new(&wasm_cfg).map_err(|e| EngineError::Init(anyhow::Error::from(e)))?;

        // WASI: preopen the current working directory as ".".
        //
        // The guest scheduler's `run(config_dir)` expects to open the provided
        // config directory path, which (in our current config) is often a
        // relative path like "./component/conf/udp-echo-minimal".
        //
        // WASI sandboxing requires that the *parent* directory is preopened.
        // Preopening "." keeps things simple and matches `wasmtime --dir=.`.
        let mut wasi_builder = WasiCtxBuilder::new();
        wasi_builder.inherit_stdio().inherit_network();
        wasi_builder
            .preopened_dir(".", ".", DirPerms::READ, FilePerms::READ)
            .map_err(|e| EngineError::Init(anyhow::Error::from(e)))?;

        let mut store = Store::new(
            &engine,
            State {
                wasi: wasi_builder.build(),
                table: wasmtime::component::ResourceTable::default(),
                rx_ring: RxRing::new(RxRingConfig::default()),
            },
        );

        let mut linker: Linker<State> = Linker::new(&engine);
        add_to_linker_sync(&mut linker).map_err(|e| EngineError::Init(anyhow::Error::from(e)))?;

        // Wire the host WIT imports (`ntx:hostnet/*`) into the component linker.
        //
        // IMPORTANT: This uses the exact bindgen-generated signature for Wasmtime 39:
        //   Hostnet::add_to_linker::<T, D>(linker, host_getter)
        // where `D` is a type that implements `HostWithStore` for the imported interfaces
        // and `host_getter` returns `D::Data<'_>`.
        packet_engine_bindings::Hostnet::add_to_linker::<
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

        let iframe_names = [
            "ntx:scenario-scheduler/scheduler-component@0.1.0",
            "scheduler-component",
        ];
        let run = find_scheduler_component_run(&iframe_names, &mut store, &instance)?;
        Ok(Self { cfg, store, run })
    }

    /// Enqueue a RX batch into the host rx-ring for the guest to pull.
    pub fn enqueue_rx_batch(&mut self, desc_mem: Vec<u8>, payload_mem: Vec<u8>) {
        self.store
            .data_mut()
            .rx_ring
            .enqueue_batch(desc_mem, payload_mem)
    }

    /// Start the guest scheduler main loop.
    ///
    /// WIT contract:
    /// - `ntx:scenario-scheduler/scheduler-component@0.1.0#run(config-dir: string) -> result<_, string>`
    ///
    /// This call is expected to block (the guest loop). The host typically runs it on a
    /// dedicated thread.
    pub fn run(&mut self, config_dir: String) -> Result<(), EngineError> {
        // In WIT, `result<_, string>` is lowered to `Result<(), String>`.
        let typed = self
            .run
            .typed::<(String,), (Result<(), String>,)>(&self.store)
            .context("run signature mismatch")
            .map_err(EngineError::Call)?;

        match typed
            .call(&mut self.store, (config_dir,))
            .context("run call")
            .map_err(EngineError::Call)?
            .0
        {
            Ok(()) => Ok(()),
            Err(e) => Err(EngineError::Call(anyhow::anyhow!(e))),
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }
}

fn find_scheduler_component_run(
    iface_names: &[&str],
    store: &mut Store<State>,
    instance: &Instance,
) -> Result<Func, EngineError> {
    let mut tried: Vec<String> = vec![];

    for iface in iface_names {
        let func_name = "run";
        tried.push(iface.to_string());
        if let Some((iface_item, iface_idx)) = instance.get_export(&mut *store, None, iface) {
            if matches!(iface_item, ComponentItem::ComponentInstance(_)) {
                tried.push(format!("{iface}::{func_name}"));
                if let Some((func_item, func_idx)) =
                    instance.get_export(&mut *store, Some(&iface_idx), func_name)
                {
                    if matches!(func_item, ComponentItem::ComponentFunc(_)) {
                        if let Some(f) = instance.get_func(&mut *store, func_idx) {
                            return Ok(f);
                        }
                    }
                }
            }
        }
    }

    Err(EngineError::EntryNotFound { candidates: tried })
}
