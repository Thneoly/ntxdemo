use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use clap::Parser;
use tokio::fs;

#[derive(Debug, Parser)]
#[command(name = "ntx-backend")]
pub struct Args {
    /// YAML config file path.
    #[arg(long, default_value = "config/ntx-backend.yaml")]
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
}

#[derive(Clone)]
pub struct AppState {
    pub data_dir: PathBuf,
    pub oras_bin: String,
    pub harbor_ca_file: Option<PathBuf>,
    pub harbor_user: Option<String>,
    pub harbor_pass: Option<String>,
    pub ingest_keep_tmp: bool,
    pub catalog_auto_ingest: bool,
    pub wasm_artifact_type: String,
}

pub async fn ensure_layout(data_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(data_dir.join("catalog")).await?;
    fs::create_dir_all(data_dir.join("config-bundles")).await?;
    fs::create_dir_all(data_dir.join("tmp")).await?;
    fs::create_dir_all(data_dir.join("wasm")).await?;
    fs::create_dir_all(data_dir.join("workflows")).await?;
    Ok(())
}
