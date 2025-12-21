use anyhow::Context;
use std::path::{Path, PathBuf};
use wasmtime::component::{Component, Func, Instance, Linker, types::ComponentItem};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::add_to_linker_sync};

use super::shared_mem;
use crate::event_bus::Bytes;

// Strongly-typed host bindings for our guest packet-engine component.
//
// This replaces string-based export lookup for the shared-memory dataplane ABI.
// Demo entrypoints (`handle-packet`, `run-scenario`) intentionally remain dynamic.
mod packet_engine_bindings {
    wasmtime::component::bindgen!({
        world: "packet-engine",
        path: "plugins/wit/packet-engine",
    });
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

struct State {
    wasi: WasiCtx,
    table: wasmtime::component::ResourceTable,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
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
            },
        );

        let mut linker: Linker<State> = Linker::new(&engine);
        add_to_linker_sync(&mut linker).map_err(|e| EngineError::Init(anyhow::Error::from(e)))?;

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
