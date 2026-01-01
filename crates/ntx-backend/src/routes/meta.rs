use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RoutesResp {
    pub name: String,
    pub version: String,
    pub git_sha: Option<String>,
    pub git_dirty: Option<bool>,
    pub routes: Vec<String>,
}

pub async fn get_routes() -> impl IntoResponse {
    let git_sha = option_env!("NTX_BACKEND_GIT_SHA").map(|s| s.to_string());
    let git_dirty = option_env!("NTX_BACKEND_GIT_DIRTY").and_then(|s| match s {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    });

    let resp = RoutesResp {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha,
        git_dirty,
        routes: crate::app::ROUTES.iter().map(|s| s.to_string()).collect(),
    };

    (StatusCode::OK, Json(resp)).into_response()
}
