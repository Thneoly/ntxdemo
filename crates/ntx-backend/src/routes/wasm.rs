use std::path::{Path, PathBuf};

use anyhow::Context;
use axum::{
    Json,
    body::Bytes,
    extract::{Multipart, Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::fs;
use tracing::error;

use crate::{
    oras::{maybe_oras_login, oras_push_files},
    respond::json_error,
    state::AppState,
    util::{looks_like_sha256_hex, registry_from_ref, wasm_path},
};

#[derive(Debug, Serialize)]
pub struct WasmEntry {
    pub sha256: String,
    pub size_bytes: u64,
    pub refs: Vec<String>,
}

fn is_catalog_cache_file_name(name: &str) -> bool {
    name.ends_with(".json")
}

async fn build_wasm_refs_index(
    data_dir: &Path,
) -> anyhow::Result<std::collections::HashMap<String, Vec<String>>> {
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let catalog_dir = data_dir.join("catalog");

    let mut rd = match fs::read_dir(&catalog_dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(map),
        Err(e) => return Err(e).context("read catalog dir"),
    };

    while let Some(ent) = rd.next_entry().await? {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_catalog_cache_file_name(file_name) {
            continue;
        }

        let s = match fs::read_to_string(&path).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let v: Value = match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let Some(wasm_sha) = v.get("wasm-sha256").and_then(|x| x.as_str()) else {
            continue;
        };
        let Some(r) = v.get("ref").and_then(|x| x.as_str()) else {
            continue;
        };
        if !looks_like_sha256_hex(wasm_sha) {
            continue;
        }

        map.entry(wasm_sha.to_string())
            .or_default()
            .push(r.to_string());
    }

    for refs in map.values_mut() {
        refs.sort();
        refs.dedup();
    }

    Ok(map)
}

pub async fn list_wasm(State(state): State<std::sync::Arc<AppState>>) -> impl IntoResponse {
    let dir = state.data_dir.join("wasm");

    let refs_index = match build_wasm_refs_index(&state.data_dir).await {
        Ok(m) => m,
        Err(e) => {
            error!(%e, "build wasm refs index failed");
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "build wasm refs index failed",
            );
        }
    };

    let mut entries: Vec<WasmEntry> = Vec::new();
    let mut rd = match fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(e) => {
            error!(%e, dir = %dir.display(), "read wasm dir failed");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "read wasm dir failed");
        }
    };

    while let Ok(Some(ent)) = rd.next_entry().await {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("wasm") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !looks_like_sha256_hex(stem) {
            continue;
        }
        let size_bytes = match ent.metadata().await {
            Ok(m) => m.len(),
            Err(_) => 0,
        };
        entries.push(WasmEntry {
            sha256: stem.to_string(),
            size_bytes,
            refs: refs_index.get(stem).cloned().unwrap_or_default(),
        });
    }

    entries.sort_by(|a, b| a.sha256.cmp(&b.sha256));
    (StatusCode::OK, Json(entries)).into_response()
}

#[derive(Debug, Serialize)]
pub struct UploadWasmResp {
    pub sha256: String,
    pub size_bytes: u64,
    pub file: String,
}

pub async fn upload_wasm(
    State(state): State<std::sync::Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    // Expect a multipart form field named "file".
    let mut wasm_bytes: Option<Vec<u8>> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        match field.bytes().await {
            Ok(b) => {
                wasm_bytes = Some(b.to_vec());
                break;
            }
            Err(e) => {
                error!(%e, "read multipart field failed");
                return json_error(StatusCode::BAD_REQUEST, "invalid multipart upload");
            }
        }
    }

    let Some(wasm_bytes) = wasm_bytes else {
        return json_error(StatusCode::BAD_REQUEST, "missing multipart field 'file'");
    };

    let mut hasher = Sha256::new();
    hasher.update(&wasm_bytes);
    let sha256_hex = hex::encode(hasher.finalize());
    let out = wasm_path(&state.data_dir, &sha256_hex);

    if let Err(e) = fs::write(&out, &wasm_bytes).await {
        error!(%e, file = %out.display(), "write wasm failed");
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "write wasm failed");
    }

    (
        StatusCode::OK,
        Json(UploadWasmResp {
            sha256: sha256_hex,
            size_bytes: wasm_bytes.len() as u64,
            file: out.display().to_string(),
        }),
    )
        .into_response()
}

pub async fn download_wasm(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath(sha256): AxumPath<String>,
) -> impl IntoResponse {
    if !looks_like_sha256_hex(&sha256) {
        return json_error(StatusCode::BAD_REQUEST, "sha256 must be 64 hex chars");
    }

    let file = wasm_path(&state.data_dir, &sha256);
    match fs::read(&file).await {
        Ok(bytes) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/wasm"),
            );
            (StatusCode::OK, headers, Bytes::from(bytes)).into_response()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json_error(StatusCode::NOT_FOUND, "wasm not found")
        }
        Err(e) => {
            error!(%e, file = %file.display(), "read wasm failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "read wasm failed")
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PushWasmBody {
    pub wasm_sha256: String,
    pub r#ref: String,

    /// If true, attach actions-catalog.json layer (generated) when pushing.
    #[serde(default)]
    pub include_catalog: bool,

    /// Override artifact type (defaults to NTX_WASM_ARTIFACT_TYPE).
    pub artifact_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PushWasmResp {
    pub r#ref: String,
    pub wasm_sha256: String,
    pub artifact_type: String,
    pub included_catalog: bool,
}

pub async fn push_wasm(
    State(state): State<std::sync::Arc<AppState>>,
    Json(body): Json<PushWasmBody>,
) -> impl IntoResponse {
    if registry_from_ref(&body.r#ref).is_none() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "ref must include registry host (e.g. host/project/repo:tag)",
        );
    }
    if !looks_like_sha256_hex(&body.wasm_sha256) {
        return json_error(StatusCode::BAD_REQUEST, "wasm_sha256 must be 64 hex chars");
    }

    let wasm_file = wasm_path(&state.data_dir, &body.wasm_sha256);
    if !fs::try_exists(&wasm_file).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "wasm not found (upload first)");
    }

    let artifact_type = body
        .artifact_type
        .unwrap_or_else(|| state.wasm_artifact_type.clone());

    // Optional login based on env config (or rely on pre-existing oras credential store).
    if let Err(e) = maybe_oras_login(&state, &body.r#ref).await {
        error!(%e, "oras login failed");
        return json_error(StatusCode::BAD_GATEWAY, format!("oras login failed: {e}"));
    }

    let tmp_dir = state
        .data_dir
        .join("tmp")
        .join(format!("push-{}", uuid::Uuid::new_v4()));
    if let Err(e) = fs::create_dir_all(&tmp_dir).await {
        error!(%e, dir = %tmp_dir.display(), "create tmp dir failed");
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "create tmp dir failed");
    }

    let wasm_copy = tmp_dir.join("component.wasm");
    if let Err(e) = fs::copy(&wasm_file, &wasm_copy).await {
        error!(%e, "copy wasm failed");
        let _ = fs::remove_dir_all(&tmp_dir).await;
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "copy wasm failed");
    }

    let mut catalog_path_opt: Option<PathBuf> = None;
    if body.include_catalog {
        match actions_catalog_gen::load_catalog_from_component(&wasm_copy).await {
            Ok(catalog) => {
                let v = match serde_json::to_value(catalog) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = fs::remove_dir_all(&tmp_dir).await;
                        return json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("serialize catalog failed: {e}"),
                        );
                    }
                };
                let catalog_file = tmp_dir.join("actions-catalog.json");
                if let Err(e) =
                    fs::write(&catalog_file, serde_json::to_string_pretty(&v).unwrap()).await
                {
                    let _ = fs::remove_dir_all(&tmp_dir).await;
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("write catalog failed: {e}"),
                    );
                }
                catalog_path_opt = Some(catalog_file);
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&tmp_dir).await;
                return json_error(
                    StatusCode::BAD_GATEWAY,
                    format!("generate catalog failed: {e}"),
                );
            }
        }
    }

    let push_result = oras_push_files(
        &state,
        &body.r#ref,
        &artifact_type,
        &wasm_copy,
        catalog_path_opt.as_deref(),
    )
    .await;

    if !state.ingest_keep_tmp {
        let _ = fs::remove_dir_all(&tmp_dir).await;
    }

    match push_result {
        Ok(()) => (
            StatusCode::OK,
            Json(PushWasmResp {
                r#ref: body.r#ref,
                wasm_sha256: body.wasm_sha256,
                artifact_type,
                included_catalog: body.include_catalog,
            }),
        )
            .into_response(),
        Err(e) => json_error(StatusCode::BAD_GATEWAY, format!("oras push failed: {e}")),
    }
}
