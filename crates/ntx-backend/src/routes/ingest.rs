use anyhow::Context;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::fs;
use tracing::error;

use crate::{
    oras::{maybe_oras_login, oras_pull_to_dir},
    respond::json_error,
    state::AppState,
    util::{catalog_path, read_first_wasm_file, registry_from_ref, wasm_path},
};

#[derive(Debug, Deserialize)]
pub struct IngestBody {
    /// OCI reference (e.g. 192.168.31.138/ntx/executor:v0.0.1)
    pub r#ref: String,

    /// If true and actions-catalog.json exists in pulled artifact, use it directly.
    #[serde(default)]
    pub prefer_published_catalog: bool,
}

#[derive(Debug, Serialize)]
pub struct IngestResp {
    pub r#ref: String,
    pub wasm_sha256: String,
    pub catalog_cache_file: String,
    pub wasm_file: String,
    pub used_published_catalog: bool,
}

pub async fn ingest(
    State(state): State<std::sync::Arc<AppState>>,
    Json(body): Json<IngestBody>,
) -> impl IntoResponse {
    let reference = body.r#ref;
    match ingest_ref(&state, &reference, body.prefer_published_catalog).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            error!(error = %e, ref_ = %reference, "ingest failed");
            json_error(StatusCode::BAD_GATEWAY, format!("ingest failed: {e}"))
        }
    }
}

pub async fn ingest_ref(
    state: &AppState,
    reference: &str,
    prefer_published_catalog: bool,
) -> anyhow::Result<IngestResp> {
    if registry_from_ref(reference).is_none() {
        anyhow::bail!("ref must include registry host (e.g. host/project/repo:tag)");
    }

    let tmp_dir = state
        .data_dir
        .join("tmp")
        .join(format!("ingest-{}", uuid::Uuid::new_v4()));

    fs::create_dir_all(&tmp_dir)
        .await
        .context("create tmp dir")?;

    let result: anyhow::Result<IngestResp> = async {
        // Optional login based on env config (or rely on pre-existing oras credential store).
        maybe_oras_login(state, reference).await?;
        oras_pull_to_dir(state, reference, &tmp_dir).await?;

        let wasm_candidate = tmp_dir.join("actions_executor.wasm");
        let wasm_file = if fs::try_exists(&wasm_candidate).await.unwrap_or(false) {
            wasm_candidate
        } else {
            read_first_wasm_file(&tmp_dir).await?
        };

        let wasm_bytes = fs::read(&wasm_file).await?;
        let mut hasher = Sha256::new();
        hasher.update(&wasm_bytes);
        let wasm_sha256_hex = hex::encode(hasher.finalize());

        let published_catalog_file = tmp_dir.join("actions-catalog.json");
        let (catalog_value, used_published_catalog) = if prefer_published_catalog
            && fs::try_exists(&published_catalog_file)
                .await
                .unwrap_or(false)
        {
            let s = fs::read_to_string(&published_catalog_file).await?;
            let v: Value =
                serde_json::from_str(&s).context("parse published actions-catalog.json")?;
            (v, true)
        } else {
            let catalog = actions_catalog_gen::load_catalog_from_component(&wasm_file).await?;
            let v = serde_json::to_value(catalog).context("serialize generated catalog")?;
            (v, false)
        };

        let catalog_cache = serde_json::json!({
            "ref": reference,
            "wasm-sha256": wasm_sha256_hex,
            "catalog": catalog_value,
        });

        let catalog_cache_file = catalog_path(state.data_dir.as_path(), reference);
        let wasm_store_file = wasm_path(
            state.data_dir.as_path(),
            catalog_cache["wasm-sha256"].as_str().unwrap(),
        );

        fs::write(
            &catalog_cache_file,
            serde_json::to_string_pretty(&catalog_cache).unwrap(),
        )
        .await
        .context("write catalog cache")?;

        fs::write(&wasm_store_file, wasm_bytes)
            .await
            .context("write wasm store")?;

        Ok(IngestResp {
            r#ref: reference.to_string(),
            wasm_sha256: catalog_cache["wasm-sha256"].as_str().unwrap().to_string(),
            catalog_cache_file: catalog_cache_file.display().to_string(),
            wasm_file: wasm_store_file.display().to_string(),
            used_published_catalog,
        })
    }
    .await;

    if !state.ingest_keep_tmp {
        let _ = fs::remove_dir_all(&tmp_dir).await;
    }

    result
}
