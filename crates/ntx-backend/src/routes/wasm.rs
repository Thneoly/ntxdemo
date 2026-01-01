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
    util::{looks_like_sha256_hex, ref_has_registry_and_repo, wasm_catalog_path, wasm_path},
};

pub async fn get_wasm_catalog(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath(sha256): AxumPath<String>,
) -> impl IntoResponse {
    if !looks_like_sha256_hex(&sha256) {
        return json_error(StatusCode::BAD_REQUEST, "sha256 must be 64 hex chars");
    }

    let wasm_file = wasm_path(&state.data_dir, &sha256);
    if !fs::try_exists(&wasm_file).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "wasm not found (upload first)");
    }

    let cached = wasm_catalog_path(&state.data_dir, &sha256);
    if fs::try_exists(&cached).await.unwrap_or(false) {
        match fs::read_to_string(&cached).await {
            Ok(s) => match serde_json::from_str::<Value>(&s) {
                Ok(v) => return (StatusCode::OK, Json(v)).into_response(),
                Err(e) => {
                    error!(%e, file = %cached.display(), "cached wasm catalog json invalid");
                    // fall through and regenerate
                }
            },
            Err(e) => {
                error!(%e, file = %cached.display(), "read cached wasm catalog failed");
                // fall through and regenerate
            }
        }
    }

    match actions_catalog_gen::load_catalog_from_component(&wasm_file).await {
        Ok(catalog) => {
            let v = match serde_json::to_value(catalog) {
                Ok(v) => v,
                Err(e) => {
                    error!(%e, "serialize wasm catalog failed");
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "serialize wasm catalog failed",
                    );
                }
            };
            let s = match serde_json::to_string_pretty(&v) {
                Ok(s) => s,
                Err(e) => {
                    error!(%e, "serialize wasm catalog failed");
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "serialize wasm catalog failed",
                    );
                }
            };
            if let Err(e) = fs::write(&cached, &s).await {
                error!(%e, file = %cached.display(), "write cached wasm catalog failed");
            }
            (StatusCode::OK, Json(v)).into_response()
        }
        Err(e) => {
            error!(%e, sha256 = %sha256, "generate wasm catalog failed");
            json_error(
                StatusCode::BAD_GATEWAY,
                format!("generate catalog failed: {e}"),
            )
        }
    }
}

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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_file: Option<String>,
}

pub async fn upload_wasm(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    // Expect a multipart form field named "file".
    let mut wasm_bytes: Option<Vec<u8>> = None;

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let content_length = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                error!(
                    %e,
                    content_type = %content_type,
                    content_length = %content_length,
                    "read multipart field failed"
                );
                return json_error(StatusCode::BAD_REQUEST, "invalid multipart upload");
            }
        };

        if field.name() != Some("file") {
            continue;
        }

        let file_name = field.file_name().map(|s| s.to_string()).unwrap_or_default();
        match field.bytes().await {
            Ok(b) => {
                wasm_bytes = Some(b.to_vec());
                break;
            }
            Err(e) => {
                error!(
                    %e,
                    content_type = %content_type,
                    content_length = %content_length,
                    file_name = %file_name,
                    "read multipart field failed"
                );
                return json_error(StatusCode::BAD_REQUEST, "invalid multipart upload");
            }
        }
    }

    let Some(wasm_bytes) = wasm_bytes else {
        if !content_type.starts_with("multipart/form-data") {
            error!(
                content_type = %content_type,
                content_length = %content_length,
                "upload_wasm called without multipart/form-data"
            );
        }
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

    // Best-effort: generate action catalog for this wasm and cache it for push.
    let catalog_file_path = wasm_catalog_path(&state.data_dir, &sha256_hex);
    let mut catalog_file_resp: Option<String> = None;
    match actions_catalog_gen::load_catalog_from_component(&out).await {
        Ok(catalog) => {
            let v = match serde_json::to_value(catalog) {
                Ok(v) => v,
                Err(e) => {
                    error!(%e, "serialize wasm catalog failed");
                    Value::Null
                }
            };
            if !v.is_null() {
                match serde_json::to_string_pretty(&v) {
                    Ok(s) => {
                        if let Err(e) = fs::write(&catalog_file_path, s).await {
                            error!(%e, file = %catalog_file_path.display(), "write wasm catalog failed");
                        } else {
                            catalog_file_resp = Some(catalog_file_path.display().to_string());
                        }
                    }
                    Err(e) => {
                        error!(%e, "serialize wasm catalog failed");
                    }
                }
            }
        }
        Err(e) => {
            // Don't fail upload if catalog generation fails; push can retry generation.
            error!(%e, "generate wasm catalog failed");
        }
    }

    (
        StatusCode::OK,
        Json(UploadWasmResp {
            sha256: sha256_hex,
            size_bytes: wasm_bytes.len() as u64,
            file: out.display().to_string(),
            catalog_file: catalog_file_resp,
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

    /// If true, attach catalog.json layer (generated/cached) when pushing.
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
    if !ref_has_registry_and_repo(&body.r#ref) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "ref must be in the form <registry>/<repo>[:tag|@digest] (no http/https scheme), e.g. 192.168.31.138/ntx/executor:v0.0.1",
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
        // Prefer the cached catalog generated at upload time.
        let cached_catalog = wasm_catalog_path(&state.data_dir, &body.wasm_sha256);
        let catalog_for_push = tmp_dir.join("catalog.json");

        if fs::try_exists(&cached_catalog).await.unwrap_or(false) {
            if let Err(e) = fs::copy(&cached_catalog, &catalog_for_push).await {
                let _ = fs::remove_dir_all(&tmp_dir).await;
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("copy cached catalog failed: {e}"),
                );
            }
            catalog_path_opt = Some(catalog_for_push);
        } else {
            // Cache-miss: generate now, write cache, and push as catalog.json.
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
                    let s = match serde_json::to_string_pretty(&v) {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = fs::remove_dir_all(&tmp_dir).await;
                            return json_error(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("serialize catalog failed: {e}"),
                            );
                        }
                    };

                    if let Err(e) = fs::write(&catalog_for_push, &s).await {
                        let _ = fs::remove_dir_all(&tmp_dir).await;
                        return json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("write catalog failed: {e}"),
                        );
                    }

                    // Best-effort cache write.
                    if let Err(e) = fs::write(&cached_catalog, &s).await {
                        error!(%e, file = %cached_catalog.display(), "write cached catalog failed");
                    }

                    catalog_path_opt = Some(catalog_for_push);
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
