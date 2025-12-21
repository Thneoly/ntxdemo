use super::{ComponentEngine, EngineConfig, EngineError, EngineResult};
use once_cell::sync::Lazy;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EngineHandle(pub String);

pub struct EngineManager {
    engines: Vec<(EngineHandle, ComponentEngine)>,
    default: Option<EngineHandle>,
}

static ENGINE_MANAGER: Lazy<Mutex<EngineManager>> = Lazy::new(|| Mutex::new(EngineManager::new()));

impl EngineManager {
    pub fn global() -> &'static Mutex<EngineManager> {
        &ENGINE_MANAGER
    }

    pub fn new() -> Self {
        Self {
            engines: vec![],
            default: None,
        }
    }

    pub fn register(&mut self, handle: EngineHandle, engine: ComponentEngine) {
        if self.default.is_none() {
            self.default = Some(handle.clone());
        }
        self.engines.push((handle, engine));
    }

    pub fn load_and_register_demo(
        &mut self,
        handle: EngineHandle,
        cfg: EngineConfig,
    ) -> Result<(), EngineError> {
        let engine = ComponentEngine::new(cfg)?;
        self.register(handle, engine);
        Ok(())
    }

    pub fn has_default(&self) -> bool {
        self.default.is_some()
    }

    pub fn tick_demo(&mut self, input: &str) -> Result<EngineResult, EngineError> {
        let Some(default) = self.default.clone() else {
            return Ok(EngineResult {
                did_work: false,
                tx: vec![],
            });
        };

        for (handle, engine) in &mut self.engines {
            if *handle == default {
                return engine.call_demo(input);
            }
        }

        Ok(EngineResult {
            did_work: false,
            tx: vec![],
        })
    }

    pub fn enqueue_rx(&mut self, sock_id: Option<u64>, payload: &[u8]) -> Result<(), EngineError> {
        let Some(default) = self.default.clone() else {
            return Ok(());
        };

        for (handle, engine) in &mut self.engines {
            if *handle == default {
                return engine.enqueue_rx(sock_id, payload);
            }
        }

        Ok(())
    }

    pub fn notify_rx(&mut self) -> Result<u32, EngineError> {
        let Some(default) = self.default.clone() else {
            return Ok(0);
        };

        for (handle, engine) in &mut self.engines {
            if *handle == default {
                return engine.notify_rx();
            }
        }

        Ok(0)
    }

    pub fn run(&mut self) -> Result<(), EngineError> {
        let Some(default) = self.default.clone() else {
            tracing::warn!(target: "ntx::wasm_engine", "EngineManager::run: no default engine configured (did you set NTX_COMPONENT?)");
            return Ok(());
        };

        for (handle, engine) in &mut self.engines {
            if *handle == default {
                tracing::info!(target: "ntx::wasm_engine", engine = %handle.0, "EngineManager::run: dispatching to default engine");
                return engine.run();
            }
        }

        tracing::warn!(target: "ntx::wasm_engine", engine = %default.0, "EngineManager::run: default engine handle not found");
        Ok(())
    }

    pub fn read_desc_buffer(&mut self) -> Result<Option<Vec<u8>>, EngineError> {
        let Some(default) = self.default.clone() else {
            return Ok(None);
        };

        for (handle, engine) in &mut self.engines {
            if *handle == default {
                return Ok(Some(engine.read_desc_buffer()?));
            }
        }

        Ok(None)
    }
}
