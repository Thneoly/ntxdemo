mod app;
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

use crate::state::{AppState, Args, ensure_layout};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,ntx_backend=debug".to_string()),
        )
        .init();

    let Args {
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
    } = args;

    ensure_layout(&data_dir).await?;

    let state = Arc::new(AppState {
        data_dir,
        oras_bin,
        harbor_ca_file,
        harbor_user,
        harbor_pass,
        ingest_keep_tmp,
        catalog_auto_ingest,
        wasm_artifact_type,
    });

    let app = app::build_app(state, cors_any_origin);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(bind = %bind, "ntx-backend listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown::shutdown_signal())
        .await?;

    Ok(())
}
