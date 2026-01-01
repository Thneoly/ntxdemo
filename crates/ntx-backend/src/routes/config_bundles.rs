use std::path::PathBuf;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{respond::json_error, state::AppState};

fn is_safe_bundle_name(name: &str) -> bool {
    // Keep it simple and safe: disallow path separators and weird traversal.
    // Allow: letters/digits/._-
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[derive(Debug, Deserialize)]
pub struct PutConfigBundleReq {
    /// Bundle directory name under `${DATA_DIR}/config-bundles/<name>/`.
    pub name: String,

    pub app_yaml: String,
    pub config_yaml: String,
    pub resources_yaml: String,
}

#[derive(Debug, Serialize)]
pub struct PutConfigBundleResp {
    pub name: String,
    pub dir: String,
    pub app_yaml_path: String,
    pub config_yaml_path: String,
    pub resources_yaml_path: String,
}

pub async fn put_config_bundle(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<PutConfigBundleReq>,
) -> impl IntoResponse {
    let name = req.name.trim().to_string();
    if !is_safe_bundle_name(&name) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid bundle name (allowed: [A-Za-z0-9._-])",
        );
    }

    if req.app_yaml.trim().is_empty()
        || req.config_yaml.trim().is_empty()
        || req.resources_yaml.trim().is_empty()
    {
        return json_error(
            StatusCode::BAD_REQUEST,
            "app_yaml/config_yaml/resources_yaml must be non-empty",
        );
    }

    let bundle_dir = state.data_dir.join("config-bundles").join(&name);
    let config_dir = bundle_dir.join("config");
    let resource_dir = config_dir.join("resource");

    if let Err(e) = fs::create_dir_all(&resource_dir).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create bundle dir failed: {e}"),
        );
    }

    let app_path: PathBuf = config_dir.join("app.yaml");
    let config_path: PathBuf = config_dir.join("config.yaml");
    let resources_path: PathBuf = resource_dir.join("resources.yaml");

    if let Err(e) = fs::write(&app_path, req.app_yaml).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write app.yaml failed: {e}"),
        );
    }
    if let Err(e) = fs::write(&config_path, req.config_yaml).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write config.yaml failed: {e}"),
        );
    }
    if let Err(e) = fs::write(&resources_path, req.resources_yaml).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write resources.yaml failed: {e}"),
        );
    }

    let resp = PutConfigBundleResp {
        name,
        dir: bundle_dir.display().to_string(),
        app_yaml_path: app_path.display().to_string(),
        config_yaml_path: config_path.display().to_string(),
        resources_yaml_path: resources_path.display().to_string(),
    };

    (StatusCode::OK, Json(resp)).into_response()
}
