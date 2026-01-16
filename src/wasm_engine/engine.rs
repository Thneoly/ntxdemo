use anyhow::Context;
use std::path::PathBuf;
use wasmtime::component::{Component, Func, Instance, Linker, types::ComponentItem};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{
    DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::add_to_linker_async,
};

use crate::kernel;
use crate::rx_ring::{RxRing, RxRingConfig};
use ntx_network;

// Strongly-typed host bindings for imports required by the composed scheduler component.
mod packet_engine_bindings {
    wasmtime::component::bindgen!({
        world: "ntx:host/hostnet",
        // This package has file-based deps under `plugins/wit/packet-engine/deps/*`.
        path: ["component/wit/net-types", "component/wit/host"],
        debug:true,
    });
}
use packet_engine_bindings::ntx::host::resources::{
    Host as ResourceHost, ResourceError, UdpIdentity,
};
use packet_engine_bindings::ntx::net_types::types::{Host as TypesHost, Ipv4Addr, MacAddr};
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
        kernel::hostnet_acquire_udp_resource(&pool, &owner)
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
        // IMPORTANT: this is a *sync* host import method (bindgen-generated today).
        // Calling `tokio::Handle::block_on` from within the Tokio runtime worker will
        // panic with "Cannot start a runtime from within a runtime".
        //
        // For now we always use the synchronous wait implementation which is backed by
        // Tokio-friendly primitives internally (it can still wake on enqueue/shutdown).
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
    stepper: Option<SchedulerStepperFuncs>,
}

#[derive(Debug, Clone, Copy)]
pub enum SchedulerStepperState {
    Uninitialized,
    Running,
    Paused,
    Stopping,
    Stopped,
    Error,
}

#[derive(Debug, Clone)]
pub struct SchedulerStepArgs {
    pub max_events: u32,
    pub max_dispatch: u32,
    pub timeout_ms: u32,
    pub now_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SchedulerStepResult {
    pub did_work: bool,
    pub processed_events: u32,
    pub dispatched: u32,
    pub rx_batches: u32,
    pub suggested_wait_ms: u32,
    pub state: SchedulerStepperState,
}

struct SchedulerStepperFuncs {
    init: Func,
    step: Func,
    request_stop: Func,
}

impl ComponentEngine {
    /// Get a clone of the host-side RX ring backing this engine.
    ///
    /// This enables the EngineOwner to accept RX batches without needing a mutable
    /// borrow of the Wasmtime `Store` (and without calling into wasm).
    pub fn rx_ring(&self) -> crate::rx_ring::RxRing {
        self.store.data().rx_ring.clone()
    }

    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    pub async fn new(cfg: EngineConfig) -> Result<Self, EngineError> {
        let mut wasm_cfg = Config::new();
        wasm_cfg.wasm_component_model(true);
        wasm_cfg.async_support(true);

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
        add_to_linker_async(&mut linker).map_err(|e| EngineError::Init(anyhow::Error::from(e)))?;

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
            .instantiate_async(&mut store, &component)
            .await
            .context("instantiate component")
            .map_err(EngineError::Init)?;

        let iframe_names = [
            "ntx:scenario-scheduler/scheduler-component@0.1.0",
            "scheduler-component",
        ];
        let run = find_scheduler_component_run(&iframe_names, &mut store, &instance)?;

        // Optional: host-driven stepper interface.
        let stepper_names = [
            "ntx:scenario-scheduler/scheduler-stepper@0.1.0",
            "scheduler-stepper",
        ];
        let stepper = find_scheduler_stepper_funcs(&stepper_names, &mut store, &instance).ok();

        Ok(Self {
            cfg,
            store,
            run,
            stepper,
        })
    }

    /// Start the guest scheduler main loop.
    ///
    /// WIT contract:
    /// - `ntx:scenario-scheduler/scheduler-component@0.1.0#run(config-dir: string) -> result<_, string>`
    ///
    /// This call is expected to block (the guest loop). The host typically runs it on a
    /// dedicated thread.
    pub async fn run(&mut self, config_dir: String) -> Result<(), EngineError> {
        // In WIT, `result<_, string>` is lowered to `Result<(), String>`.
        let typed = self
            .run
            .typed::<(String,), (Result<(), String>,)>(&self.store)
            .context("run signature mismatch")
            .map_err(EngineError::Call)?;

        match typed
            .call_async(&mut self.store, (config_dir,))
            .await
            .context("run call")
            .map_err(EngineError::Call)?
            .0
        {
            Ok(()) => Ok(()),
            Err(e) => Err(EngineError::Call(anyhow::anyhow!(e))),
        }
    }

    pub fn supports_stepper(&self) -> bool {
        self.stepper.is_some()
    }

    pub async fn init_stepper(&mut self, config_dir: String) -> Result<(), EngineError> {
        let Some(stepper) = self.stepper.as_ref() else {
            return Err(EngineError::EntryNotFound {
                candidates: vec![
                    "ntx:scenario-scheduler/scheduler-stepper@0.1.0#init".to_string(),
                    "scheduler-stepper#init".to_string(),
                ],
            });
        };

        let typed = stepper
            .init
            .typed::<(String,), (Result<(), String>,)>(&self.store)
            .context("init signature mismatch")
            .map_err(EngineError::Call)?;

        match typed
            .call_async(&mut self.store, (config_dir,))
            .await
            .context("init call")
            .map_err(EngineError::Call)?
            .0
        {
            Ok(()) => Ok(()),
            Err(e) => Err(EngineError::Call(anyhow::anyhow!(e))),
        }
    }

    pub async fn request_stop_stepper(&mut self) -> Result<(), EngineError> {
        let Some(stepper) = self.stepper.as_ref() else {
            return Err(EngineError::EntryNotFound {
                candidates: vec![
                    "ntx:scenario-scheduler/scheduler-stepper@0.1.0#request-stop".to_string(),
                    "scheduler-stepper#request-stop".to_string(),
                ],
            });
        };

        let typed = stepper
            .request_stop
            .typed::<(), (Result<(), String>,)>(&self.store)
            .context("request-stop signature mismatch")
            .map_err(EngineError::Call)?;

        match typed
            .call_async(&mut self.store, ())
            .await
            .context("request-stop call")
            .map_err(EngineError::Call)?
            .0
        {
            Ok(()) => Ok(()),
            Err(e) => Err(EngineError::Call(anyhow::anyhow!(e))),
        }
    }

    pub async fn step_stepper(
        &mut self,
        args: SchedulerStepArgs,
    ) -> Result<SchedulerStepResult, EngineError> {
        use wasmtime::component::Val;

        let Some(stepper) = self.stepper.as_ref() else {
            return Err(EngineError::EntryNotFound {
                candidates: vec![
                    "ntx:scenario-scheduler/scheduler-stepper@0.1.0#step".to_string(),
                    "scheduler-stepper#step".to_string(),
                ],
            });
        };

        // Dynamic call to avoid tightly coupling host to record/enum ABI details.
        // Records are represented as (field_name, value) pairs.
        let now_val = Val::Option(args.now_ms.map(|v| Box::new(Val::U64(v))));
        let arg0 = Val::Record(vec![
            ("max-events".to_string(), Val::U32(args.max_events)),
            ("max-dispatch".to_string(), Val::U32(args.max_dispatch)),
            ("timeout-ms".to_string(), Val::U32(args.timeout_ms)),
            ("now-ms".to_string(), now_val),
        ]);

        let result_arity = stepper.step.ty(&self.store).results().len();
        let mut results: Vec<Val> = Vec::with_capacity(result_arity);
        if result_arity == 1 {
            // result<step-result, string>
            let ok_payload = Val::Record(vec![
                ("did-work".to_string(), Val::Bool(false)),
                ("processed-events".to_string(), Val::U32(0)),
                ("dispatched".to_string(), Val::U32(0)),
                ("rx-batches".to_string(), Val::U32(0)),
                ("suggested-wait-ms".to_string(), Val::U32(0)),
                ("state".to_string(), Val::Enum("uninitialized".to_string())),
            ]);
            results.push(Val::Result(Ok(Some(Box::new(ok_payload)))));
        } else {
            for _ in 0..result_arity {
                results.push(Val::Result(Ok(None)));
            }
        }
        stepper
            .step
            .call_async(&mut self.store, &[arg0], &mut results)
            .await
            .context("step call")
            .map_err(EngineError::Call)?;

        // Expected result is a single `result<step-result, string>`.
        if results.len() != 1 {
            return Err(EngineError::Call(anyhow::anyhow!(
                "step returned unexpected result arity: {}",
                results.len()
            )));
        }
        let first = results.remove(0);

        decode_step_result(first)
    }
}

fn decode_step_result(v: wasmtime::component::Val) -> Result<SchedulerStepResult, EngineError> {
    use std::collections::HashMap;
    use wasmtime::component::Val;

    // Expect: result<step-result,string>
    let Val::Result(r) = v else {
        return Err(EngineError::Call(anyhow::anyhow!(
            "unexpected step result value: {:?}",
            v
        )));
    };

    match r {
        Ok(Some(okv)) => {
            let Val::Record(fields) = *okv else {
                return Err(EngineError::Call(anyhow::anyhow!(
                    "unexpected ok step result: {:?}",
                    okv
                )));
            };

            let mut map: HashMap<String, Val> = fields.into_iter().collect();

            let did_work = match map.remove("did-work") {
                Some(Val::Bool(b)) => b,
                other => {
                    return Err(EngineError::Call(anyhow::anyhow!(
                        "bad did-work: {:?}",
                        other
                    )));
                }
            };
            let processed_events = match map.remove("processed-events") {
                Some(Val::U32(n)) => n,
                other => {
                    return Err(EngineError::Call(anyhow::anyhow!(
                        "bad processed-events: {:?}",
                        other
                    )));
                }
            };
            let dispatched = match map.remove("dispatched") {
                Some(Val::U32(n)) => n,
                other => {
                    return Err(EngineError::Call(anyhow::anyhow!(
                        "bad dispatched: {:?}",
                        other
                    )));
                }
            };
            let rx_batches = match map.remove("rx-batches") {
                Some(Val::U32(n)) => n,
                other => {
                    return Err(EngineError::Call(anyhow::anyhow!(
                        "bad rx-batches: {:?}",
                        other
                    )));
                }
            };
            let suggested_wait_ms = match map.remove("suggested-wait-ms") {
                Some(Val::U32(n)) => n,
                other => {
                    return Err(EngineError::Call(anyhow::anyhow!(
                        "bad suggested-wait-ms: {:?}",
                        other
                    )));
                }
            };
            let state = match map.remove("state") {
                Some(Val::Enum(name)) => match name.as_str() {
                    "uninitialized" => SchedulerStepperState::Uninitialized,
                    "running" => SchedulerStepperState::Running,
                    "paused" => SchedulerStepperState::Paused,
                    "stopping" => SchedulerStepperState::Stopping,
                    "stopped" => SchedulerStepperState::Stopped,
                    "error" => SchedulerStepperState::Error,
                    other => {
                        return Err(EngineError::Call(anyhow::anyhow!(
                            "unknown scheduler-state: {other}"
                        )));
                    }
                },
                other => return Err(EngineError::Call(anyhow::anyhow!("bad state: {:?}", other))),
            };

            Ok(SchedulerStepResult {
                did_work,
                processed_events,
                dispatched,
                rx_batches,
                suggested_wait_ms,
                state,
            })
        }
        Ok(None) => Err(EngineError::Call(anyhow::anyhow!(
            "step returned ok without value"
        ))),
        Err(Some(errv)) => {
            let msg = match *errv {
                Val::String(s) => s,
                other => format!("{:?}", other),
            };
            Err(EngineError::Call(anyhow::anyhow!(msg)))
        }
        Err(None) => Err(EngineError::Call(anyhow::anyhow!(
            "step returned err without value"
        ))),
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

fn find_scheduler_stepper_funcs(
    iface_names: &[&str],
    store: &mut Store<State>,
    instance: &Instance,
) -> Result<SchedulerStepperFuncs, EngineError> {
    let mut tried: Vec<String> = vec![];

    for iface in iface_names {
        tried.push(iface.to_string());
        if let Some((iface_item, iface_idx)) = instance.get_export(&mut *store, None, iface) {
            if !matches!(iface_item, ComponentItem::ComponentInstance(_)) {
                continue;
            }

            let init = {
                let func_name = "init";
                tried.push(format!("{iface}::{func_name}"));
                let Some((func_item, func_idx)) =
                    instance.get_export(&mut *store, Some(&iface_idx), func_name)
                else {
                    continue;
                };
                if !matches!(func_item, ComponentItem::ComponentFunc(_)) {
                    continue;
                }
                instance.get_func(&mut *store, func_idx).ok_or_else(|| {
                    EngineError::EntryNotFound {
                        candidates: tried.clone(),
                    }
                })?
            };

            let step = {
                let func_name = "step";
                tried.push(format!("{iface}::{func_name}"));
                let Some((func_item, func_idx)) =
                    instance.get_export(&mut *store, Some(&iface_idx), func_name)
                else {
                    continue;
                };
                if !matches!(func_item, ComponentItem::ComponentFunc(_)) {
                    continue;
                }
                instance.get_func(&mut *store, func_idx).ok_or_else(|| {
                    EngineError::EntryNotFound {
                        candidates: tried.clone(),
                    }
                })?
            };

            let request_stop = {
                let func_name = "request-stop";
                tried.push(format!("{iface}::{func_name}"));
                let Some((func_item, func_idx)) =
                    instance.get_export(&mut *store, Some(&iface_idx), func_name)
                else {
                    continue;
                };
                if !matches!(func_item, ComponentItem::ComponentFunc(_)) {
                    continue;
                }
                instance.get_func(&mut *store, func_idx).ok_or_else(|| {
                    EngineError::EntryNotFound {
                        candidates: tried.clone(),
                    }
                })?
            };

            return Ok(SchedulerStepperFuncs {
                init,
                step,
                request_stop,
            });
        }
    }

    Err(EngineError::EntryNotFound { candidates: tried })
}
