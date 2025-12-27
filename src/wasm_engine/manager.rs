use super::{ComponentEngine, EngineConfig, EngineError};
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

    pub fn load_and_register(
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

    pub fn notify_rx(
        &mut self,
        desc_mem: Vec<u8>,
        payload_mem: Vec<u8>,
    ) -> Result<u32, EngineError> {
        let Some(default) = self.default.clone() else {
            return Ok(0);
        };

        for (handle, engine) in &mut self.engines {
            if *handle == default {
                return engine.notify_rx(desc_mem, payload_mem);
            }
        }

        Ok(0)
    }
}
