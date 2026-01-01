use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Parser;
use tokio::fs;
use tokio::{process::Child, sync::Mutex};

#[derive(Debug, Parser)]
#[command(name = "ntx-backend")]
pub struct Args {
    /// YAML config file path.
    #[arg(long, default_value = "crates/ntx-backend/conf/ntx-backend.yaml")]
    pub config: PathBuf,

    /// Bind address.
    #[arg(long)]
    pub bind: Option<SocketAddr>,

    /// Data directory for persisted artifacts (catalog/workflows).
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Allow all CORS origins (dev default).
    #[arg(long)]
    pub cors_any_origin: Option<bool>,

    /// Path to oras binary.
    #[arg(long)]
    pub oras_bin: Option<String>,

    /// Default Harbor/registry CA file for self-signed TLS.
    #[arg(long)]
    pub harbor_ca_file: Option<PathBuf>,

    /// Default Harbor/registry username (optional; can also pre-login with oras).
    #[arg(long)]
    pub harbor_user: Option<String>,

    /// Default Harbor/registry password (optional; used with --password-stdin).
    #[arg(long)]
    pub harbor_pass: Option<String>,

    /// Keep ingest temp directories under data_dir/tmp (debugging).
    #[arg(long)]
    pub ingest_keep_tmp: Option<bool>,

    /// When GET /api/v1/catalog cache-misses, automatically ingest from registry.
    #[arg(long)]
    pub catalog_auto_ingest: Option<bool>,

    /// Default artifact type when pushing wasm to Harbor.
    #[arg(long)]
    pub wasm_artifact_type: Option<String>,

    /// Path to `ntx-wac-compose` binary.
    #[arg(long)]
    pub wac_compose_bin: Option<String>,

    /// Working directory used to run `wac compose` (repo root).
    ///
    /// If unset, the backend will try to auto-detect by walking up from current_dir.
    #[arg(long)]
    pub wac_compose_cwd: Option<PathBuf>,

    /// Path to `ntx` binary used for run-bundles execution.
    #[arg(long)]
    pub ntx_bin: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
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

    /// In-memory process table for run-bundles.
    ///
    /// Minimal lifecycle management: start, status, stop, logs.
    pub run_processes: Arc<Mutex<HashMap<String, RunProcess>>>,
}

pub struct RunProcess {
    pub child: Child,
    pub run_dir: PathBuf,
    pub command: Vec<String>,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
}

pub async fn ensure_layout(data_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(data_dir.join("catalog")).await?;
    fs::create_dir_all(data_dir.join("config-bundles")).await?;
    fs::create_dir_all(data_dir.join("run-bundles")).await?;
    fs::create_dir_all(data_dir.join("tmp")).await?;
    fs::create_dir_all(data_dir.join("wasm")).await?;
    fs::create_dir_all(data_dir.join("workflows")).await?;
    Ok(())
}
