use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HarborConfig {
    #[serde(default)]
    pub ca_file: Option<PathBuf>,

    #[serde(default)]
    pub user: Option<String>,

    #[serde(default)]
    pub pass: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BackendConfigFile {
    #[serde(default)]
    pub bind: Option<SocketAddr>,

    #[serde(default)]
    pub data_dir: Option<PathBuf>,

    #[serde(default)]
    pub cors_any_origin: Option<bool>,

    #[serde(default)]
    pub oras_bin: Option<String>,

    #[serde(default)]
    pub harbor: Option<HarborConfig>,

    #[serde(default)]
    pub ingest_keep_tmp: Option<bool>,

    #[serde(default)]
    pub catalog_auto_ingest: Option<bool>,

    #[serde(default)]
    pub wasm_artifact_type: Option<String>,

    /// Path to `ntx-wac-compose` binary.
    #[serde(default)]
    pub wac_compose_bin: Option<String>,

    /// Working directory used to run `wac compose` (repo root).
    #[serde(default)]
    pub wac_compose_cwd: Option<PathBuf>,

    /// Path to `ntx` binary used for run-bundles execution.
    #[serde(default)]
    pub ntx_bin: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub cors_any_origin: bool,
    pub oras_bin: String,
    pub harbor_ca_file: Option<PathBuf>,
    pub harbor_user: Option<String>,
    pub harbor_pass: Option<String>,
    pub ingest_keep_tmp: bool,
    pub catalog_auto_ingest: bool,
    pub wasm_artifact_type: String,
    pub wac_compose_bin: String,
    pub wac_compose_cwd: Option<PathBuf>,

    pub ntx_bin: String,
}

impl BackendConfig {
    pub fn defaults() -> anyhow::Result<Self> {
        Ok(Self {
            bind: "127.0.0.1:9090".parse().context("parse default bind")?,
            data_dir: PathBuf::from("./.ntx-backend"),
            cors_any_origin: true,
            oras_bin: "oras".to_string(),
            harbor_ca_file: None,
            harbor_user: None,
            harbor_pass: None,
            ingest_keep_tmp: false,
            catalog_auto_ingest: false,
            wasm_artifact_type: "application/vnd.ntx.action-executor.v1".to_string(),
            wac_compose_bin: "ntx-wac-compose".to_string(),
            wac_compose_cwd: None,
            ntx_bin: "ntx".to_string(),
        })
    }

    pub fn apply_file(mut self, cfg: BackendConfigFile) -> Self {
        if let Some(v) = cfg.bind {
            self.bind = v;
        }
        if let Some(v) = cfg.data_dir {
            self.data_dir = v;
        }
        if let Some(v) = cfg.cors_any_origin {
            self.cors_any_origin = v;
        }
        if let Some(v) = cfg.oras_bin {
            self.oras_bin = v;
        }
        if let Some(v) = cfg.ingest_keep_tmp {
            self.ingest_keep_tmp = v;
        }
        if let Some(v) = cfg.catalog_auto_ingest {
            self.catalog_auto_ingest = v;
        }
        if let Some(v) = cfg.wasm_artifact_type {
            self.wasm_artifact_type = v;
        }

        if let Some(v) = cfg.wac_compose_bin {
            self.wac_compose_bin = v;
        }
        if let Some(v) = cfg.wac_compose_cwd {
            self.wac_compose_cwd = Some(v);
        }

        if let Some(v) = cfg.ntx_bin {
            self.ntx_bin = v;
        }

        if let Some(h) = cfg.harbor {
            if h.ca_file.is_some() {
                self.harbor_ca_file = h.ca_file;
            }
            if h.user.is_some() {
                self.harbor_user = h.user;
            }
            if h.pass.is_some() {
                self.harbor_pass = h.pass;
            }
        }

        self
    }

    pub fn apply_cli_overrides(
        mut self,
        bind: Option<SocketAddr>,
        data_dir: Option<PathBuf>,
        cors_any_origin: Option<bool>,
        oras_bin: Option<String>,
        harbor_ca_file: Option<PathBuf>,
        harbor_user: Option<String>,
        harbor_pass: Option<String>,
        ingest_keep_tmp: Option<bool>,
        catalog_auto_ingest: Option<bool>,
        wasm_artifact_type: Option<String>,
        wac_compose_bin: Option<String>,
        wac_compose_cwd: Option<PathBuf>,
        ntx_bin: Option<String>,
    ) -> Self {
        if let Some(v) = bind {
            self.bind = v;
        }
        if let Some(v) = data_dir {
            self.data_dir = v;
        }
        if let Some(v) = cors_any_origin {
            self.cors_any_origin = v;
        }
        if let Some(v) = oras_bin {
            self.oras_bin = v;
        }
        if let Some(v) = harbor_ca_file {
            self.harbor_ca_file = Some(v);
        }
        if let Some(v) = harbor_user {
            self.harbor_user = Some(v);
        }
        if let Some(v) = harbor_pass {
            self.harbor_pass = Some(v);
        }
        if let Some(v) = ingest_keep_tmp {
            self.ingest_keep_tmp = v;
        }
        if let Some(v) = catalog_auto_ingest {
            self.catalog_auto_ingest = v;
        }
        if let Some(v) = wasm_artifact_type {
            self.wasm_artifact_type = v;
        }
        if let Some(v) = wac_compose_bin {
            self.wac_compose_bin = v;
        }
        if let Some(v) = wac_compose_cwd {
            self.wac_compose_cwd = Some(v);
        }
        if let Some(v) = ntx_bin {
            self.ntx_bin = v;
        }
        self
    }
}

pub fn load_config_file(path: &Path) -> anyhow::Result<BackendConfigFile> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read config file {}", path.display()))?;
    let cfg: BackendConfigFile =
        serde_yaml::from_slice(&bytes).with_context(|| format!("parse yaml {}", path.display()))?;
    Ok(cfg)
}
