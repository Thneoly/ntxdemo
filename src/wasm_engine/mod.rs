pub mod engine;
pub mod manager;
// Kept for backward compatibility with older call sites.
pub mod shared_mem {
    pub use crate::rx_layout::*;
}

pub use engine::{ComponentEngine, EngineConfig, EngineError};
pub use manager::{EngineHandle, EngineManager};
