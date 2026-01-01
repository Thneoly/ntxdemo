use std::path::{Path, PathBuf};

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_yaml::Value as YamlValue;
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

#[derive(Debug, Serialize)]
pub struct ConfigBundleSummary {
    pub name: String,
    pub dir: String,
    pub app_yaml_path: String,
}

#[derive(Debug, Serialize)]
pub struct SchedulerWasmConfigExtract {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    pub entry_candidates: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GetConfigBundleResp {
    pub name: String,
    pub dir: String,
    pub app_yaml_path: String,
    pub config_yaml_path: String,
    pub resources_yaml_path: String,

    pub app_yaml: String,
    pub config_yaml: String,
    pub resources_yaml: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduler_wasm: Option<SchedulerWasmConfigExtract>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduler_wasm_parse_error: Option<String>,
}

fn bundle_paths(data_dir: &Path, name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let bundle_dir = data_dir.join("config-bundles").join(name);
    let config_dir = bundle_dir.join("config");
    let app_path: PathBuf = config_dir.join("app.yaml");
    let config_path: PathBuf = config_dir.join("config.yaml");
    let resources_path: PathBuf = config_dir.join("resource").join("resources.yaml");
    (bundle_dir, app_path, config_path, resources_path)
}

fn yaml_get_mapping<'a>(v: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    match v {
        YamlValue::Mapping(m) => m.get(YamlValue::String(key.to_string())),
        _ => None,
    }
}

fn yaml_get_string(v: &YamlValue) -> Option<String> {
    v.as_str().map(|s| s.to_string())
}

fn yaml_get_string_seq(v: &YamlValue) -> Vec<String> {
    match v {
        YamlValue::Sequence(seq) => seq
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

fn extract_scheduler_wasm(app_yaml: &str) -> Result<Option<SchedulerWasmConfigExtract>, String> {
    let root: YamlValue = serde_yaml::from_str(app_yaml).map_err(|e| e.to_string())?;
    let Some(scheduler) = yaml_get_mapping(&root, "scheduler") else {
        return Ok(None);
    };
    let Some(wasm) = yaml_get_mapping(scheduler, "wasm") else {
        return Ok(None);
    };

    let component_path = yaml_get_mapping(wasm, "component_path").and_then(yaml_get_string);
    let config_dir = yaml_get_mapping(wasm, "config_dir").and_then(yaml_get_string);
    let entry_candidates = yaml_get_mapping(wasm, "entry_candidates")
        .map(yaml_get_string_seq)
        .unwrap_or_default();

    Ok(Some(SchedulerWasmConfigExtract {
        component_path,
        config_dir,
        entry_candidates,
    }))
}

pub async fn list_config_bundles(
    State(state): State<std::sync::Arc<AppState>>,
) -> impl IntoResponse {
    let dir = state.data_dir.join("config-bundles");

    let mut rd = match fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (StatusCode::OK, Json(Vec::<ConfigBundleSummary>::new())).into_response();
        }
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read config-bundles dir failed: {e}"),
            );
        }
    };

    let mut out: Vec<ConfigBundleSummary> = Vec::new();
    while let Ok(Some(ent)) = rd.next_entry().await {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
        else {
            continue;
        };
        if !is_safe_bundle_name(&name) {
            continue;
        }
        let (bundle_dir, app_path, _config_path, _resources_path) =
            bundle_paths(&state.data_dir, &name);
        out.push(ConfigBundleSummary {
            name,
            dir: bundle_dir.display().to_string(),
            app_yaml_path: app_path.display().to_string(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));

    (StatusCode::OK, Json(out)).into_response()
}

pub async fn get_config_bundle(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    let name = name.trim().to_string();
    if !is_safe_bundle_name(&name) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid bundle name (allowed: [A-Za-z0-9._-])",
        );
    }

    let (bundle_dir, app_path, config_path, resources_path) = bundle_paths(&state.data_dir, &name);

    if !fs::try_exists(&bundle_dir).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "bundle not found");
    }

    let app_yaml = match fs::read_to_string(&app_path).await {
        Ok(s) => s,
        Err(e) => {
            return json_error(StatusCode::NOT_FOUND, format!("read app.yaml failed: {e}"));
        }
    };
    let config_yaml = match fs::read_to_string(&config_path).await {
        Ok(s) => s,
        Err(e) => {
            return json_error(
                StatusCode::NOT_FOUND,
                format!("read config.yaml failed: {e}"),
            );
        }
    };
    let resources_yaml = match fs::read_to_string(&resources_path).await {
        Ok(s) => s,
        Err(e) => {
            return json_error(
                StatusCode::NOT_FOUND,
                format!("read resources.yaml failed: {e}"),
            );
        }
    };

    let (scheduler_wasm, scheduler_wasm_parse_error) = match extract_scheduler_wasm(&app_yaml) {
        Ok(v) => (v, None),
        Err(e) => (None, Some(e)),
    };

    let resp = GetConfigBundleResp {
        name,
        dir: bundle_dir.display().to_string(),
        app_yaml_path: app_path.display().to_string(),
        config_yaml_path: config_path.display().to_string(),
        resources_yaml_path: resources_path.display().to_string(),
        app_yaml,
        config_yaml,
        resources_yaml,
        scheduler_wasm,
        scheduler_wasm_parse_error,
    };

    (StatusCode::OK, Json(resp)).into_response()
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
