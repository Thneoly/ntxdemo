use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use clap::Parser;
use tokio::fs;

#[derive(Debug, Parser)]
#[command(name = "ntx-backend")]
pub struct Args {
    /// Bind address.
    #[arg(long, env = "NTX_BACKEND_BIND", default_value = "127.0.0.1:8080")]
    pub bind: SocketAddr,

    /// Data directory for persisted artifacts (catalog/workflows).
    #[arg(long, env = "NTX_BACKEND_DATA_DIR", default_value = "./.ntx-backend")]
    pub data_dir: PathBuf,

    /// Allow all CORS origins (dev default).
    #[arg(long, env = "NTX_BACKEND_CORS_ANY_ORIGIN", default_value_t = true)]
    pub cors_any_origin: bool,

    /// Path to oras binary.
    #[arg(long, env = "NTX_ORAS_BIN", default_value = "oras")]
    pub oras_bin: String,

    /// Default Harbor/registry CA file for self-signed TLS.
    #[arg(long, env = "NTX_HARBOR_CA_FILE")]
    pub harbor_ca_file: Option<PathBuf>,

    /// Default Harbor/registry username (optional; can also pre-login with oras).
    #[arg(long, env = "NTX_HARBOR_USER")]
    pub harbor_user: Option<String>,

    /// Default Harbor/registry password (optional; used with --password-stdin).
    #[arg(long, env = "NTX_HARBOR_PASS")]
    pub harbor_pass: Option<String>,

    /// Keep ingest temp directories under data_dir/tmp (debugging).
    #[arg(long, env = "NTX_INGEST_KEEP_TMP", default_value_t = false)]
    pub ingest_keep_tmp: bool,

    /// When GET /api/v1/catalog cache-misses, automatically ingest from registry.
    #[arg(long, env = "NTX_CATALOG_AUTO_INGEST", default_value_t = false)]
    pub catalog_auto_ingest: bool,

    /// Default artifact type when pushing wasm to Harbor.
    #[arg(
        long,
        env = "NTX_WASM_ARTIFACT_TYPE",
        default_value = "application/vnd.ntx.action-executor.v1"
    )]
    pub wasm_artifact_type: String,
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
    fs::create_dir_all(data_dir.join("tmp")).await?;
    fs::create_dir_all(data_dir.join("wasm")).await?;
    fs::create_dir_all(data_dir.join("workflows")).await?;
    Ok(())
}
