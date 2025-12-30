use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    root: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct PublishPayload {
    manifest: serde_json::Value,
    release: serde_json::Value,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let root = std::env::var("REGISTRY_ROOT").unwrap_or_else(|_| "./registry-data".to_string());
    let state = AppState { root: root.into() };

    let app = Router::new()
        // reads
        .route("/plugins/:name/:version/manifest.json", get(get_manifest))
        .route("/plugins/:name/:version/release.json", get(get_release))
        .route("/plugins/:name/:version/component.wasm", get(get_component))
        .route(
            "/plugins/by-digest/sha256/:hex/component.wasm",
            get(get_component_by_digest),
        )
        // writes
        .route("/plugins/:name/:version", post(publish_version))
        .route("/plugins/:name/:version/yank", post(yank_version))
        .with_state(state);

    let addr: SocketAddr = std::env::var("REGISTRY_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()?;
    tracing::info!(%addr, "registry server starting");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn get_manifest(
    State(st): State<AppState>,
    Path((name, version)): Path<(String, String)>,
) -> impl IntoResponse {
    let p = st
        .root
        .join("plugins")
        .join(&name)
        .join(&version)
        .join("manifest.json");
    match fs::read(p).await {
        Ok(b) => (StatusCode::OK, axum::body::Bytes::from(b)).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_release(
    State(st): State<AppState>,
    Path((name, version)): Path<(String, String)>,
) -> impl IntoResponse {
    let p = st
        .root
        .join("plugins")
        .join(&name)
        .join(&version)
        .join("release.json");
    match fs::read(p).await {
        Ok(b) => (StatusCode::OK, axum::body::Bytes::from(b)).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_component(
    State(st): State<AppState>,
    Path((name, version)): Path<(String, String)>,
) -> impl IntoResponse {
    let p = st
        .root
        .join("plugins")
        .join(&name)
        .join(&version)
        .join("component.wasm");
    match fs::read(p).await {
        Ok(b) => (StatusCode::OK, axum::body::Bytes::from(b)).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_component_by_digest(
    State(st): State<AppState>,
    Path(hex): Path<String>,
) -> impl IntoResponse {
    let p = st
        .root
        .join("plugins/by-digest/sha256")
        .join(&hex)
        .join("component.wasm");
    match fs::read(p).await {
        Ok(b) => (StatusCode::OK, axum::body::Bytes::from(b)).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn publish_version(
    State(st): State<AppState>,
    Path((name, version)): Path<(String, String)>,
    mut mp: Multipart,
) -> impl IntoResponse {
    let res: Result<()> = async move {
        let vdir = st.root.join("plugins").join(&name).join(&version);
        fs::create_dir_all(&vdir)
            .await
            .context("mkdir version dir")?;

        let mut manifest: Option<serde_json::Value> = None;
        let mut release: Option<serde_json::Value> = None;
        let mut component_bytes: Option<Vec<u8>> = None;

        while let Some(field) = mp.next_field().await? {
            let fname = field.file_name().map(|s| s.to_string()).unwrap_or_default();
            let name = field.name().map(|s| s.to_string()).unwrap_or_default();
            let data = field.bytes().await?.to_vec();
            match (name.as_str(), fname.as_str()) {
                ("manifest.json", _) => manifest = Some(serde_json::from_slice(&data)?),
                ("release.json", _) => release = Some(serde_json::from_slice(&data)?),
                ("component.wasm", _) => component_bytes = Some(data),
                _ => {}
            }
        }

        let manifest = manifest.context("missing manifest.json")?;
        let release = release.context("missing release.json")?;
        let component = component_bytes.context("missing component.wasm")?;

        fs::write(
            vdir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )
        .await?;
        fs::write(
            vdir.join("release.json"),
            serde_json::to_vec_pretty(&release)?,
        )
        .await?;
        fs::write(vdir.join("component.wasm"), &component).await?;

        if let Some(sha) = release
            .pointer("/component/sha256")
            .and_then(|v| v.as_str())
        {
            let bd = st
                .root
                .join("plugins/by-digest/sha256")
                .join(sha)
                .join("component.wasm");
            if let Some(parent) = bd.parent() {
                fs::create_dir_all(parent).await.ok();
            }
            if fs::hard_link(vdir.join("component.wasm"), &bd)
                .await
                .is_err()
            {
                fs::copy(vdir.join("component.wasm"), &bd).await.ok();
            }
        }

        Ok(())
    }
    .await;

    match res {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "publish failed");
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
    }
}

async fn yank_version(
    State(st): State<AppState>,
    Path((name, version)): Path<(String, String)>,
) -> impl IntoResponse {
    let res: Result<()> = async move {
        let p = st
            .root
            .join("plugins")
            .join(&name)
            .join(&version)
            .join(".yanked");
        fs::write(p, b"yanked").await?;
        Ok(())
    }
    .await;

    match res {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "yank failed");
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
    }
}
