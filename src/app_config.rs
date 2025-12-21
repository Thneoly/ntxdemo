use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Unified application configuration.
///
/// This config is intentionally small and focused: it centralizes both kernel
/// and scheduler/wasm setup so we don't rely on environment variables.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub kernel: KernelConfig,

    #[serde(default)]
    pub scheduler: SchedulerConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KernelConfig {
    /// Path to the kernel YAML config (NIC + resource pools).
    ///
    /// Defaults to `config/config.yaml` for backwards compatibility.
    #[serde(default = "default_kernel_config_path")]
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SchedulerConfig {
    #[serde(default)]
    pub wasm: WasmConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WasmConfig {
    /// Optional wasm component path to auto-load at scheduler startup.
    #[serde(default)]
    pub component_path: Option<PathBuf>,

    /// Candidate export entrypoints to look for.
    #[serde(default = "default_entry_candidates")]
    pub entry_candidates: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            kernel: KernelConfig {
                config_path: default_kernel_config_path(),
            },
            scheduler: SchedulerConfig {
                wasm: WasmConfig {
                    component_path: None,
                    entry_candidates: default_entry_candidates(),
                },
            },
        }
    }
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            config_path: default_kernel_config_path(),
        }
    }
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            component_path: None,
            entry_candidates: default_entry_candidates(),
        }
    }
}

fn default_kernel_config_path() -> PathBuf {
    PathBuf::from("config/config.yaml")
}

fn default_entry_candidates() -> Vec<String> {
    vec!["handle-packet".into(), "run-scenario".into(), "run".into()]
}

impl AppConfig {
    pub fn load_yaml_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read app config file: {}", path.display()))?;
        let mut cfg: Self = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse app yaml config: {}", path.display()))?;

        // Defensive: if user provides an empty list explicitly, keep sane defaults.
        if cfg.scheduler.wasm.entry_candidates.is_empty() {
            cfg.scheduler.wasm.entry_candidates = default_entry_candidates();
        }
        Ok(cfg)
    }
}
