use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;
use tracing::error;

use crate::{respond::json_error, state::AppState, util::catalog_path};

use super::ingest::ingest_ref;

#[derive(Debug, Deserialize)]
pub struct CatalogQuery {
    /// OCI reference (e.g. 192.168.31.138/ntx/executor:v0.0.1)
    pub r#ref: String,

    /// If true, when catalog cache misses, server will try to ingest automatically.
    #[serde(default)]
    pub auto_ingest: bool,
}

#[derive(Debug, Deserialize)]
pub struct PutCatalogBody {
    /// OCI reference.
    pub r#ref: String,

    /// The catalog JSON payload.
    pub catalog: Value,
}

pub async fn get_catalog(
    State(state): State<std::sync::Arc<AppState>>,
    Query(q): Query<CatalogQuery>,
) -> impl IntoResponse {
    let path = catalog_path(&state.data_dir, &q.r#ref);

    match fs::read_to_string(&path).await {
        Ok(s) => match serde_json::from_str::<Value>(&s) {
            Ok(json) => (StatusCode::OK, Json(json)).into_response(),
            Err(e) => {
                error!(%e, file = %path.display(), "invalid catalog json");
                json_error(StatusCode::INTERNAL_SERVER_ERROR, "catalog json is invalid")
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if q.auto_ingest || state.catalog_auto_ingest {
                match ingest_ref(&state, &q.r#ref, true).await {
                    Ok(_) => match fs::read_to_string(&path).await {
                        Ok(s) => match serde_json::from_str::<Value>(&s) {
                            Ok(json) => (StatusCode::OK, Json(json)).into_response(),
                            Err(e) => {
                                error!(%e, file = %path.display(), "invalid catalog json after ingest");
                                json_error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "catalog json is invalid",
                                )
                            }
                        },
                        Err(e) => {
                            error!(%e, file = %path.display(), "read catalog failed after ingest");
                            json_error(StatusCode::INTERNAL_SERVER_ERROR, "read catalog failed")
                        }
                    },
                    Err(e) => {
                        json_error(StatusCode::BAD_GATEWAY, format!("auto-ingest failed: {e}"))
                    }
                }
            } else {
                json_error(
                    StatusCode::NOT_FOUND,
                    "catalog not found (ingest/generate first)",
                )
            }
        }
        Err(e) => {
            error!(%e, file = %path.display(), "read catalog failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "read catalog failed")
        }
    }
}

pub async fn put_catalog(
    State(state): State<std::sync::Arc<AppState>>,
    Json(body): Json<PutCatalogBody>,
) -> impl IntoResponse {
    let path = catalog_path(&state.data_dir, &body.r#ref);

    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent).await {
            error!(%e, dir = %parent.display(), "create dir failed");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "create dir failed");
        }
    }

    let s = match serde_json::to_string_pretty(&body.catalog) {
        Ok(s) => s,
        Err(_) => {
            return json_error(StatusCode::BAD_REQUEST, "catalog must be valid json");
        }
    };

    match fs::write(&path, s).await {
        Ok(()) => (StatusCode::OK, "ok").into_response(),
        Err(e) => {
            error!(%e, file = %path.display(), "write catalog failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "write catalog failed")
        }
    }
}
