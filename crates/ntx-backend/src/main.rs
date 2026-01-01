mod app;
mod config;
mod oras;
mod respond;
mod routes;
mod shutdown;
mod state;
mod types;
mod util;

use std::sync::Arc;

use clap::Parser;
use tracing::info;

use crate::{
    config::{BackendConfig, load_config_file},
    state::{AppState, Args, ensure_layout},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,ntx_backend=debug".to_string()),
        )
        .init();

    let Args {
        config,
        bind,
        data_dir,
        cors_any_origin,
        oras_bin,
        harbor_ca_file,
        harbor_user,
        harbor_pass,
        ingest_keep_tmp,
        catalog_auto_ingest,
        wasm_artifact_type,
        wac_compose_bin,
        wac_compose_cwd,
    } = args;

    let file_cfg = load_config_file(&config)?;
    let cfg = BackendConfig::defaults()?
        .apply_file(file_cfg)
        .apply_cli_overrides(
            bind,
            data_dir,
            cors_any_origin,
            oras_bin,
            harbor_ca_file,
            harbor_user,
            harbor_pass,
            ingest_keep_tmp,
            catalog_auto_ingest,
            wasm_artifact_type,
            wac_compose_bin,
            wac_compose_cwd,
        );

    ensure_layout(&cfg.data_dir).await?;

    let state = Arc::new(AppState {
        data_dir: cfg.data_dir,
        oras_bin: cfg.oras_bin,
        harbor_ca_file: cfg.harbor_ca_file,
        harbor_user: cfg.harbor_user,
        harbor_pass: cfg.harbor_pass,
        ingest_keep_tmp: cfg.ingest_keep_tmp,
        catalog_auto_ingest: cfg.catalog_auto_ingest,
        wasm_artifact_type: cfg.wasm_artifact_type,
        wac_compose_bin: cfg.wac_compose_bin,
        wac_compose_cwd: cfg.wac_compose_cwd,
    });

    let app = app::build_app(state, cfg.cors_any_origin);

    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    let git_sha = option_env!("NTX_BACKEND_GIT_SHA");
    let git_dirty = option_env!("NTX_BACKEND_GIT_DIRTY");
    info!(
        bind = %cfg.bind,
        config = %config.display(),
        name = env!("CARGO_PKG_NAME"),
        version = env!("CARGO_PKG_VERSION"),
        git_sha = git_sha.unwrap_or(""),
        git_dirty = git_dirty.unwrap_or(""),
        "ntx-backend listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown::shutdown_signal())
        .await?;

    Ok(())
}
