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

    pub async fn load_and_register_async(
        &mut self,
        handle: EngineHandle,
        cfg: EngineConfig,
    ) -> Result<(), EngineError> {
        let engine = ComponentEngine::new(cfg).await?;
        self.register(handle, engine);
        Ok(())
    }

    pub fn has_default(&self) -> bool {
        self.default.is_some()
    }

    /// Move the default engine out of the manager.
    ///
    /// This is used by the host "engine owner" task so that the long-running
    /// `scheduler-component.run()` is not executed while holding the global
    /// `EngineManager` mutex.
    pub fn take_default_engine(&mut self) -> ComponentEngine {
        let default = self
            .default
            .clone()
            .expect("default engine not set; did you forget to register?");

        let idx = self
            .engines
            .iter()
            .position(|(h, _)| *h == default)
            .expect("default engine missing from registry");

        self.engines.swap_remove(idx).1
    }
}
