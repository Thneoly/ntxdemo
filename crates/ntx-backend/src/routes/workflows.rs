use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    extract::Path as AxumPath,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;
use tracing::error;

use crate::{
    respond::json_error,
    state::AppState,
    util::workflow_path,
};

#[derive(Debug, Deserialize)]
pub struct CreateWorkflowBody {
    /// If omitted, server will generate one.
    pub id: Option<String>,

    /// Graph payload (frontend draft JSON).
    pub graph: Value,
}

#[derive(Debug, Serialize)]
pub struct CreateWorkflowResp {
    pub id: String,
}

pub async fn create_workflow(
    State(state): State<std::sync::Arc<AppState>>,
    Json(body): Json<CreateWorkflowBody>,
) -> impl IntoResponse {
    let id = body.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let path = workflow_path(&state.data_dir, &id);

    let payload = serde_json::json!({
        "id": id,
        "graph": body.graph,
    });

    match fs::write(&path, serde_json::to_string_pretty(&payload).unwrap()).await {
        Ok(()) => (StatusCode::OK, Json(CreateWorkflowResp { id })).into_response(),
        Err(e) => {
            error!(%e, file = %path.display(), "write workflow failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "write workflow failed")
        }
    }
}

pub async fn get_workflow(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    let path = workflow_path(&state.data_dir, &id);

    match fs::read_to_string(&path).await {
        Ok(s) => match serde_json::from_str::<Value>(&s) {
            Ok(json) => (StatusCode::OK, Json(json)).into_response(),
            Err(e) => {
                error!(%e, file = %path.display(), "invalid workflow json");
                json_error(StatusCode::INTERNAL_SERVER_ERROR, "workflow json is invalid")
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json_error(StatusCode::NOT_FOUND, "workflow not found")
        }
        Err(e) => {
            error!(%e, file = %path.display(), "read workflow failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "read workflow failed")
        }
    }
}
