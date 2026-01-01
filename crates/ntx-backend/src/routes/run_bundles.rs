use std::{
    path::Component,
    path::{Path, PathBuf},
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_yaml::Value as YamlValue;
use tokio::fs;
use tokio::fs::OpenOptions;
use tokio::process::Command;

use crate::{
    respond::json_error,
    state::AppState,
    util::{
        looks_like_sha256_hex, run_bundle_dir, wasm_catalog_path, wasm_path,
        wasm_scheduler_composed_path,
    },
};

fn find_repo_root() -> Option<std::path::PathBuf> {
    // Prefer deriving from the backend executable path:
    //   <repo>/target/<profile>/ntx-backend
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.as_path();
        while let Some(parent) = cur.parent() {
            if cur.file_name().map(|n| n == "target").unwrap_or(false) {
                return Some(parent.to_path_buf());
            }
            cur = parent;
        }
    }

    // Fallback: walk up from cwd until we find Cargo.toml.
    let mut cur = std::env::current_dir().ok()?;
    loop {
        if cur.join("Cargo.toml").is_file() {
            return Some(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

fn resolve_ntx_bin(raw: &str, backend_config_path: Option<&Path>) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "ntx".to_string();
    }

    // If it's already an absolute path, just use it.
    let p = std::path::Path::new(raw);
    if p.is_absolute() {
        return raw.to_string();
    }

    // If it exists relative to current working directory, use it.
    if p.exists() {
        return raw.to_string();
    }

    // If it exists relative to backend config file directory, use it.
    if let Some(cfg_path) = backend_config_path {
        if let Some(cfg_dir) = cfg_path.parent() {
            let candidate = cfg_dir.join(p);
            if candidate.exists() {
                return candidate.display().to_string();
            }
        }
    }

    // If it looks like a repo-relative path, resolve against repo root.
    if let Some(root) = find_repo_root() {
        let candidate = root.join(p);
        if candidate.exists() {
            return candidate.display().to_string();
        }
    }

    raw.to_string()
}

fn is_safe_id(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn config_bundle_dir(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join("config-bundles").join(name)
}

async fn ensure_parent_dir(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    Ok(())
}

fn yaml_mapping_mut(v: &mut YamlValue) -> &mut serde_yaml::Mapping {
    if !matches!(v, YamlValue::Mapping(_)) {
        *v = YamlValue::Mapping(serde_yaml::Mapping::new());
    }
    match v {
        YamlValue::Mapping(m) => m,
        _ => unreachable!(),
    }
}

fn yaml_set_string(root: &mut YamlValue, path: &[&str], value: String) {
    let mut cur = root;
    for (i, key) in path.iter().enumerate() {
        let is_last = i == path.len() - 1;
        let m = yaml_mapping_mut(cur);
        let k = YamlValue::String((*key).to_string());
        if is_last {
            m.insert(k, YamlValue::String(value));
            return;
        }

        if !m.contains_key(&k) {
            m.insert(k.clone(), YamlValue::Mapping(serde_yaml::Mapping::new()));
        }
        cur = m.get_mut(&k).expect("just inserted");
    }
}

fn to_abs_string(path: &Path) -> String {
    if path.is_absolute() {
        return path.display().to_string();
    }

    if let Some(root) = find_repo_root() {
        return root.join(path).display().to_string();
    }

    if let Ok(cwd) = std::env::current_dir() {
        return cwd.join(path).display().to_string();
    }

    path.display().to_string()
}

fn yaml_get_string<'a>(root: &'a YamlValue, path: &[&str]) -> Option<&'a str> {
    let mut cur = root;
    for key in path {
        let YamlValue::Mapping(m) = cur else {
            return None;
        };
        cur = m.get(&YamlValue::String((*key).to_string()))?;
    }
    match cur {
        YamlValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

fn yaml_is_mapping(root: &YamlValue, path: &[&str]) -> bool {
    let mut cur = root;
    for key in path {
        let YamlValue::Mapping(m) = cur else {
            return false;
        };
        cur = match m.get(&YamlValue::String((*key).to_string())) {
            Some(v) => v,
            None => return false,
        };
    }
    matches!(cur, YamlValue::Mapping(_))
}

fn yaml_get_bool(root: &YamlValue, path: &[&str]) -> Option<bool> {
    let mut cur = root;
    for key in path {
        let YamlValue::Mapping(m) = cur else {
            return None;
        };
        cur = m.get(&YamlValue::String((*key).to_string()))?;
    }
    match cur {
        YamlValue::Bool(b) => Some(*b),
        _ => None,
    }
}

fn ensure_rel_no_parent(dir: &str) -> bool {
    let p = Path::new(dir);
    if p.is_absolute() {
        return false;
    }
    !p.components().any(|c| matches!(c, Component::ParentDir))
}

fn is_safe_filename(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let p = Path::new(name);
    p.components().all(|c| match c {
        Component::Normal(_) => true,
        _ => false,
    })
}

fn html_escape(s: &str) -> String {
    // Minimal escaping for embedding filesystem paths / names into HTML.
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

async fn resolve_run_bundle_capture_dir(run_dir: &Path) -> anyhow::Result<PathBuf> {
    let config_yaml_path = run_dir.join("config").join("config.yaml");
    let config_base_dir = config_yaml_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| run_dir.join("config"));

    let raw = match fs::read_to_string(&config_yaml_path).await {
        Ok(s) => s,
        Err(e) => {
            // If config.yaml is missing, still default to a conventional location.
            if e.kind() == std::io::ErrorKind::NotFound {
                return Ok(config_base_dir.join("captures"));
            }
            return Err(e.into());
        }
    };

    let v: YamlValue = serde_yaml::from_str(&raw)?;
    // Runtime reads capture config at the top-level of config.yaml:
    //   capture:
    //     enabled: true
    //     dir: "./pcap"
    // Keep backward-compat for older variants under kernel.capture.*
    let dir_raw = yaml_get_string(&v, &["capture", "dir"])
        .or_else(|| yaml_get_string(&v, &["kernel", "capture", "dir"]));

    // If capture.dir is not set, default to run-bundle output directory.
    let Some(dir_raw) = dir_raw else {
        return Ok(run_dir.join("output").join("pcap"));
    };

    // Mirror the runtime behavior: relative capture.dir resolves relative to config.yaml directory.
    // But for safety, disallow `..` traversal in the relative path.
    if Path::new(dir_raw).is_absolute() {
        Ok(PathBuf::from(dir_raw))
    } else {
        if !ensure_rel_no_parent(dir_raw) {
            anyhow::bail!("invalid capture.dir (must not contain '..')");
        }
        Ok(config_base_dir.join(dir_raw))
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateRunBundleReq {
    /// Optional bundle id. If omitted, backend generates one.
    #[serde(default)]
    pub id: Option<String>,

    /// Existing config bundle name under `${DATA_DIR}/config-bundles/<name>/`.
    pub config_bundle: String,

    /// Existing uploaded wasm sha256 (hex) under `${DATA_DIR}/wasm/<sha>.wasm`.
    pub wasm_sha256: String,

    /// Scenario YAML generated by the builder.
    pub scenario_yaml: String,
}

#[derive(Debug, Serialize)]
pub struct CreateRunBundleResp {
    pub id: String,
    pub dir: String,
    pub config_dir: String,
    pub scenario_yaml_path: String,
    pub wasm_path: String,
    pub catalog_path: String,
    pub scheduler_composed_wasm_path: String,
}

#[derive(Debug, Serialize)]
pub struct RunRunBundleResp {
    pub id: String,
    pub pid: u32,
    pub command: Vec<String>,
    pub run_dir: String,
    pub stdout_path: String,
    pub stderr_path: String,
}

#[derive(Debug, Serialize)]
pub struct RunBundleStatusResp {
    pub id: String,
    pub running: bool,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub command: Option<Vec<String>>,
    pub run_dir: String,
    pub stdout_path: String,
    pub stderr_path: String,
}

#[derive(Debug, Serialize)]
pub struct RunBundleStopResp {
    pub id: String,
    pub stopped: bool,
}

#[derive(Debug, Serialize)]
pub struct RunBundleLogsResp {
    pub id: String,
    pub stdout: String,
    pub stderr: String,
    pub stdout_path: String,
    pub stderr_path: String,
    pub truncated: bool,
}

async fn read_tail_string(path: &Path, max_bytes: usize) -> anyhow::Result<(String, bool)> {
    let bytes = match fs::read(path).await {
        Ok(b) => b,
        Err(e) => {
            // If file doesn't exist yet, treat as empty.
            if e.kind() == std::io::ErrorKind::NotFound {
                return Ok((String::new(), false));
            }
            return Err(e.into());
        }
    };
    if bytes.len() <= max_bytes {
        Ok((String::from_utf8_lossy(&bytes).to_string(), false))
    } else {
        let start = bytes.len() - max_bytes;
        Ok((String::from_utf8_lossy(&bytes[start..]).to_string(), true))
    }
}

async fn check_ntx_has_required_caps(ntx_path: &Path) -> anyhow::Result<Option<bool>> {
    // Returns:
    // - Ok(Some(true))  => getcap succeeded and required caps are present
    // - Ok(Some(false)) => getcap succeeded and required caps are missing
    // - Ok(None)        => getcap is unavailable or failed; cannot verify
    let out = match Command::new("getcap").arg(ntx_path).output().await {
        Ok(o) => o,
        Err(_e) => return Ok(None),
    };
    if !out.status.success() {
        return Ok(None);
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let ok = s.contains("cap_net_raw") && s.contains("cap_net_admin") && s.contains("=ep");
    Ok(Some(ok))
}

pub async fn create_run_bundle(
    State(state): State<std::sync::Arc<AppState>>,
    Json(body): Json<CreateRunBundleReq>,
) -> impl IntoResponse {
    let config_bundle = body.config_bundle.trim().to_string();
    if !is_safe_id(&config_bundle) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid config_bundle (allowed: [A-Za-z0-9._-])",
        );
    }

    let wasm_sha256 = body.wasm_sha256.trim().to_lowercase();
    if !looks_like_sha256_hex(&wasm_sha256) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid wasm_sha256 (expected 64 hex chars)",
        );
    }

    let id = match body.id {
        Some(v) => v.trim().to_string(),
        None => {
            let ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            format!("run-{ms}")
        }
    };
    if !is_safe_id(&id) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid id (allowed: [A-Za-z0-9._-])",
        );
    }

    let source_bundle_dir = config_bundle_dir(&state.data_dir, &config_bundle);
    if !fs::try_exists(&source_bundle_dir).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "config bundle not found");
    }

    let source_wasm = wasm_path(&state.data_dir, &wasm_sha256);
    if !fs::try_exists(&source_wasm).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "wasm not found (upload it first)");
    }

    let source_catalog = wasm_catalog_path(&state.data_dir, &wasm_sha256);
    if !fs::try_exists(&source_catalog).await.unwrap_or(false) {
        return json_error(
            StatusCode::NOT_FOUND,
            "catalog.json not found for this wasm (upload wasm with catalog generation enabled)",
        );
    }

    let source_composed = wasm_scheduler_composed_path(&state.data_dir, &wasm_sha256);
    if !fs::try_exists(&source_composed).await.unwrap_or(false) {
        return json_error(
            StatusCode::NOT_FOUND,
            "scheduler-composed.wasm not found for this wasm (ensure wac compose ran during upload/push)",
        );
    }

    let out_dir = run_bundle_dir(&state.data_dir, &id);
    if fs::try_exists(&out_dir).await.unwrap_or(false) {
        return json_error(StatusCode::CONFLICT, "run bundle id already exists");
    }

    // Create runnable layout aligned with `ntx --config <dir>` behavior.
    // run-bundles/<id>/
    //   config/
    //     app.yaml            (rewritten to use absolute paths)
    //     config.yaml
    //     resource/resources.yaml
    //     scenario.yaml
    //   wasm/
    //     executor.wasm
    //     catalog.json
    //     scheduler-composed.wasm
    //   output/
    //     logs/
    //     pcap/
    let out_config_dir = out_dir.join("config");
    let out_output_dir = out_dir.join("output");
    let out_output_pcap_dir = out_output_dir.join("pcap");
    let out_scenario = out_config_dir.join("scenario.yaml");
    let out_wasm = out_dir.join("wasm").join("executor.wasm");
    let out_catalog = out_dir.join("wasm").join("catalog.json");
    let out_composed = out_dir.join("wasm").join("scheduler-composed.wasm");

    if let Err(e) = fs::create_dir_all(&out_dir).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create run bundle dir failed: {e}"),
        );
    }

    if let Err(e) = fs::create_dir_all(&out_output_dir).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create output dir failed: {e}"),
        );
    }

    // Copy config files (explicit list keeps it simple and deterministic).
    // Source files:
    //   config/app.yaml
    //   config/config.yaml
    //   config/resource/resources.yaml
    let src_config_dir = source_bundle_dir.join("config");
    let src_app = src_config_dir.join("app.yaml");
    let src_cfg = src_config_dir.join("config.yaml");
    let src_res = src_config_dir.join("resource").join("resources.yaml");

    let dst_app = out_config_dir.join("app.yaml");
    let dst_cfg = out_config_dir.join("config.yaml");
    let dst_res = out_config_dir.join("resource").join("resources.yaml");

    for (src, dst, label) in [
        (src_app, dst_app, "app.yaml"),
        (src_cfg, dst_cfg, "config.yaml"),
        (src_res, dst_res, "resources.yaml"),
    ] {
        if !fs::try_exists(&src).await.unwrap_or(false) {
            return json_error(
                StatusCode::NOT_FOUND,
                format!("config bundle missing {label}"),
            );
        }
        if let Err(e) = ensure_parent_dir(&dst).await {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create config dir failed: {e}"),
            );
        }
        if let Err(e) = fs::copy(&src, &dst).await {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("copy {label} failed: {e}"),
            );
        }
    }

    if let Err(e) = fs::write(&out_scenario, body.scenario_yaml).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write scenario.yaml failed: {e}"),
        );
    }

    if let Err(e) = ensure_parent_dir(&out_wasm).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create wasm dir failed: {e}"),
        );
    }
    if let Err(e) = fs::copy(&source_wasm, &out_wasm).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("copy wasm failed: {e}"),
        );
    }

    if let Err(e) = fs::copy(&source_catalog, &out_catalog).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("copy catalog.json failed: {e}"),
        );
    }

    if let Err(e) = fs::copy(&source_composed, &out_composed).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("copy scheduler-composed.wasm failed: {e}"),
        );
    }

    // Rewrite app.yaml so users can run from anywhere:
    //   ntx --config <run-bundle-dir>
    // The host uses cfg.kernel.config_path and cfg.scheduler.wasm.* as-is.
    let dst_app = out_config_dir.join("app.yaml");
    let app_raw = match fs::read_to_string(&dst_app).await {
        Ok(s) => s,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read packaged app.yaml failed: {e}"),
            );
        }
    };
    let mut app_yaml: YamlValue = match serde_yaml::from_str(&app_raw) {
        Ok(v) => v,
        Err(e) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                format!("parse packaged app.yaml failed: {e}"),
            );
        }
    };

    yaml_set_string(
        &mut app_yaml,
        &["kernel", "config_path"],
        to_abs_string(&out_config_dir.join("config.yaml")),
    );
    yaml_set_string(
        &mut app_yaml,
        &["scheduler", "wasm", "component_path"],
        to_abs_string(&out_composed),
    );
    yaml_set_string(
        &mut app_yaml,
        &["scheduler", "wasm", "config_dir"],
        "config".to_string(),
    );

    let new_app_raw = match serde_yaml::to_string(&app_yaml) {
        Ok(s) => s,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize app.yaml failed: {e}"),
            );
        }
    };
    if let Err(e) = fs::write(&dst_app, new_app_raw).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write packaged app.yaml failed: {e}"),
        );
    }

    // Rewrite config.yaml resource path to be absolute as well.
    // Some configs use relative paths like "config/resource/resources.yaml".
    // Make it robust regardless of process cwd.
    let dst_cfg = out_config_dir.join("config.yaml");
    let cfg_raw = match fs::read_to_string(&dst_cfg).await {
        Ok(s) => s,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read packaged config.yaml failed: {e}"),
            );
        }
    };
    let mut cfg_yaml: YamlValue = match serde_yaml::from_str(&cfg_raw) {
        Ok(v) => v,
        Err(e) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                format!("parse packaged config.yaml failed: {e}"),
            );
        }
    };
    yaml_set_string(
        &mut cfg_yaml,
        &["resource", "path"],
        to_abs_string(&out_config_dir.join("resource").join("resources.yaml")),
    );

    // Allow container deployments to override the AF_PACKET interface name.
    // Host installs typically use `ntx0`, but containers usually have `eth0`.
    if let Ok(iface) = std::env::var("NTX_KERNEL_IFACE") {
        let iface = iface.trim();
        if !iface.is_empty() {
            yaml_set_string(&mut cfg_yaml, &["nic", "iface"], iface.to_string());
        }
    }

    // Ensure capture output ends up under run-bundle output/pcap.
    // If capture is enabled (or capture section exists), set capture.dir to an absolute path.
    let capture_enabled = yaml_get_bool(&cfg_yaml, &["capture", "enabled"]).unwrap_or(false);
    let capture_has_dir = yaml_get_string(&cfg_yaml, &["capture", "dir"]).is_some();
    let capture_has_section = yaml_is_mapping(&cfg_yaml, &["capture"]);

    if capture_enabled || capture_has_dir || capture_has_section {
        // Create the pcap directory early so users can see it immediately under output/.
        let _ = fs::create_dir_all(&out_output_pcap_dir).await;
        yaml_set_string(
            &mut cfg_yaml,
            &["capture", "dir"],
            to_abs_string(&out_output_pcap_dir),
        );
    }
    let new_cfg_raw = match serde_yaml::to_string(&cfg_yaml) {
        Ok(s) => s,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize config.yaml failed: {e}"),
            );
        }
    };
    if let Err(e) = fs::write(&dst_cfg, new_cfg_raw).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write packaged config.yaml failed: {e}"),
        );
    }

    (
        StatusCode::OK,
        Json(CreateRunBundleResp {
            id: id.clone(),
            dir: out_dir.display().to_string(),
            config_dir: out_config_dir.display().to_string(),
            scenario_yaml_path: out_scenario.display().to_string(),
            wasm_path: out_wasm.display().to_string(),
            catalog_path: out_catalog.display().to_string(),
            scheduler_composed_wasm_path: out_composed.display().to_string(),
        }),
    )
        .into_response()
}

pub async fn run_run_bundle(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let id = id.trim().to_string();
    if !is_safe_id(&id) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid id (allowed: [A-Za-z0-9._-])",
        );
    }

    let run_dir = run_bundle_dir(&state.data_dir, &id);
    if !fs::try_exists(&run_dir).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "run bundle not found");
    }

    // Ntx accepts either a config file path or a directory; when we also set
    // `current_dir` to `run_dir`, a relative `--config` argument would resolve
    // relative to `run_dir` (and likely break). Always pass an absolute path.
    let run_dir_abs: PathBuf = std::fs::canonicalize(&run_dir).unwrap_or_else(|_| run_dir.clone());

    // If already running, return a conflict.
    {
        let mut procs = state.run_processes.lock().await;
        if let Some(existing) = procs.get_mut(&id) {
            match existing.child.try_wait() {
                Ok(Some(_status)) => {
                    // Process already exited; allow re-run.
                    procs.remove(&id);
                }
                Ok(None) => {
                    return json_error(StatusCode::CONFLICT, "run bundle already running");
                }
                Err(e) => {
                    procs.remove(&id);
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("check running process failed: {e}"),
                    );
                }
            }
        }
    }

    let log_dir = run_dir.join("output").join("logs");
    if let Err(e) = fs::create_dir_all(&log_dir).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create logs dir failed: {e}"),
        );
    }
    let stdout_path = log_dir.join("ntx.stdout.log");
    let stderr_path = log_dir.join("ntx.stderr.log");

    let stdout_file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("open stdout log failed: {e}"),
            );
        }
    };
    let stderr_file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("open stderr log failed: {e}"),
            );
        }
    };

    let stdout_std = stdout_file.into_std().await;
    let stderr_std = stderr_file.into_std().await;

    // Resolve `ntx` binary path.
    // If `ntx` is not on PATH (common in dev), prefer a repo-local build output.
    let resolved_ntx_bin = resolve_ntx_bin(&state.ntx_bin, Some(&state.config_path));

    // Ensure required capabilities for AF_PACKET execution.
    // Root-friendly behavior: do NOT attempt to call `setcap` automatically.
    // Instead, detect missing caps (when possible) and return an actionable hint.
    let resolved_path = std::path::Path::new(&resolved_ntx_bin);
    if resolved_path.exists() {
        match check_ntx_has_required_caps(resolved_path).await {
            Ok(Some(true)) => {}
            Ok(Some(false)) => {
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "ntx binary is missing required Linux capabilities for AF_PACKET (CAP_NET_RAW + CAP_NET_ADMIN): {}. Fix once with: sudo setcap cap_net_raw,cap_net_admin+ep {} (note: rebuilding the binary clears caps)",
                        resolved_path.display(),
                        resolved_path.display(),
                    ),
                );
            }
            Ok(None) | Err(_) => {
                // Can't verify (getcap missing or failed). Let it run; if it fails with
                // EPERM, the user will see the same setcap hint in logs/docs.
            }
        }
    }

    let cmd_vec = vec![
        resolved_ntx_bin.clone(),
        "--config".to_string(),
        run_dir_abs.display().to_string(),
    ];

    let mut cmd = Command::new(&resolved_ntx_bin);
    cmd.arg("--config")
        .arg(&run_dir_abs)
        .current_dir(&run_dir_abs)
        .stdout(Stdio::from(stdout_std))
        .stderr(Stdio::from(stderr_std));

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let exists = std::path::Path::new(&resolved_ntx_bin).exists();
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "spawn ntx failed: {e}. ntx_bin(raw)={:?} ntx_bin(resolved)={:?} exists={} backend_config={}. Hint: set backend `ntx_bin` to an absolute path, or use a path relative to the backend config file directory, or start backend with `--ntx-bin /abs/path/to/ntx`",
                    state.ntx_bin,
                    resolved_ntx_bin,
                    exists,
                    state.config_path.display(),
                ),
            );
        }
    };

    let pid = match child.id() {
        Some(pid) => pid,
        None => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "spawned process had no pid",
            );
        }
    };

    {
        let mut procs = state.run_processes.lock().await;
        procs.insert(
            id.clone(),
            crate::state::RunProcess {
                child,
                run_dir: run_dir.clone(),
                command: cmd_vec.clone(),
                stdout_path: stdout_path.clone(),
                stderr_path: stderr_path.clone(),
            },
        );
    }

    (
        StatusCode::OK,
        Json(RunRunBundleResp {
            id,
            pid,
            command: cmd_vec,
            run_dir: run_dir.display().to_string(),
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
        }),
    )
        .into_response()
}

pub async fn get_run_bundle_status(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let id = id.trim().to_string();
    if !is_safe_id(&id) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid id (allowed: [A-Za-z0-9._-])",
        );
    }

    let run_dir = run_bundle_dir(&state.data_dir, &id);
    if !fs::try_exists(&run_dir).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "run bundle not found");
    }

    let stdout_path = run_dir.join("output").join("logs").join("ntx.stdout.log");
    let stderr_path = run_dir.join("output").join("logs").join("ntx.stderr.log");

    let mut procs = state.run_processes.lock().await;
    if let Some(mut proc) = procs.remove(&id) {
        match proc.child.try_wait() {
            Ok(Some(status)) => {
                let exit_code = status.code();
                return (
                    StatusCode::OK,
                    Json(RunBundleStatusResp {
                        id,
                        running: false,
                        pid: None,
                        exit_code,
                        command: Some(proc.command),
                        run_dir: run_dir.display().to_string(),
                        stdout_path: proc.stdout_path.display().to_string(),
                        stderr_path: proc.stderr_path.display().to_string(),
                    }),
                )
                    .into_response();
            }
            Ok(None) => {
                let pid = proc.child.id();
                let command = proc.command.clone();
                let stdout_path = proc.stdout_path.clone();
                let stderr_path = proc.stderr_path.clone();
                procs.insert(id.clone(), proc);
                return (
                    StatusCode::OK,
                    Json(RunBundleStatusResp {
                        id,
                        running: true,
                        pid,
                        exit_code: None,
                        command: Some(command),
                        run_dir: run_dir.display().to_string(),
                        stdout_path: stdout_path.display().to_string(),
                        stderr_path: stderr_path.display().to_string(),
                    }),
                )
                    .into_response();
            }
            Err(e) => {
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("check running process failed: {e}"),
                );
            }
        }
    }

    // Not tracked in memory => not running (or started elsewhere).
    (
        StatusCode::OK,
        Json(RunBundleStatusResp {
            id,
            running: false,
            pid: None,
            exit_code: None,
            command: None,
            run_dir: run_dir.display().to_string(),
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
        }),
    )
        .into_response()
}

pub async fn stop_run_bundle(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let id = id.trim().to_string();
    if !is_safe_id(&id) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid id (allowed: [A-Za-z0-9._-])",
        );
    }

    let mut procs = state.run_processes.lock().await;
    let Some(mut proc) = procs.remove(&id) else {
        return (
            StatusCode::OK,
            Json(RunBundleStopResp { id, stopped: false }),
        )
            .into_response();
    };

    let _ = proc.child.kill().await;
    (
        StatusCode::OK,
        Json(RunBundleStopResp { id, stopped: true }),
    )
        .into_response()
}

pub async fn get_run_bundle_logs(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let id = id.trim().to_string();
    if !is_safe_id(&id) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid id (allowed: [A-Za-z0-9._-])",
        );
    }

    let run_dir = run_bundle_dir(&state.data_dir, &id);
    if !fs::try_exists(&run_dir).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "run bundle not found");
    }

    // Prefer tracked paths (if process was started via this backend).
    let (stdout_path, stderr_path) = {
        let procs = state.run_processes.lock().await;
        if let Some(proc) = procs.get(&id) {
            (proc.stdout_path.clone(), proc.stderr_path.clone())
        } else {
            (
                run_dir.join("output").join("logs").join("ntx.stdout.log"),
                run_dir.join("output").join("logs").join("ntx.stderr.log"),
            )
        }
    };

    let max_bytes = 20_000usize;
    let (stdout, t1) = match read_tail_string(&stdout_path, max_bytes).await {
        Ok(v) => v,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read stdout logs failed: {e}"),
            );
        }
    };
    let (stderr, t2) = match read_tail_string(&stderr_path, max_bytes).await {
        Ok(v) => v,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read stderr logs failed: {e}"),
            );
        }
    };

    (
        StatusCode::OK,
        Json(RunBundleLogsResp {
            id,
            stdout,
            stderr,
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
            truncated: t1 || t2,
        }),
    )
        .into_response()
}

#[derive(Debug, Serialize)]
pub struct RunBundleCaptureFile {
    pub name: String,
    pub size_bytes: u64,
    pub modified_ms: Option<u128>,
}

#[derive(Debug, Serialize)]
pub struct RunBundleCapturesResp {
    pub id: String,
    pub capture_dir: String,
    pub files: Vec<RunBundleCaptureFile>,
}

pub async fn list_run_bundle_captures(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let id = id.trim().to_string();
    if !is_safe_id(&id) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid id (allowed: [A-Za-z0-9._-])",
        );
    }

    let run_dir = run_bundle_dir(&state.data_dir, &id);
    if !fs::try_exists(&run_dir).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "run bundle not found");
    }

    let capture_dir = match resolve_run_bundle_capture_dir(&run_dir).await {
        Ok(p) => p,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("resolve capture dir failed: {e}"),
            );
        }
    };

    let mut files: Vec<RunBundleCaptureFile> = Vec::new();
    let mut rd = match fs::read_dir(&capture_dir).await {
        Ok(rd) => rd,
        Err(e) => {
            // No captures yet => empty list.
            if e.kind() == std::io::ErrorKind::NotFound {
                return (
                    StatusCode::OK,
                    Json(RunBundleCapturesResp {
                        id,
                        capture_dir: capture_dir.display().to_string(),
                        files,
                    }),
                )
                    .into_response();
            }
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read capture dir failed: {e}"),
            );
        }
    };

    while let Ok(Some(ent)) = rd.next_entry().await {
        let name = ent.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pcap") {
            continue;
        }
        let meta = match ent.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis()));
        files.push(RunBundleCaptureFile {
            name,
            size_bytes: meta.len(),
            modified_ms,
        });
    }

    // Newest first.
    files.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));

    (
        StatusCode::OK,
        Json(RunBundleCapturesResp {
            id,
            capture_dir: capture_dir.display().to_string(),
            files,
        }),
    )
        .into_response()
}

pub async fn download_run_bundle_capture_latest(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let id = id.trim().to_string();
    if !is_safe_id(&id) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid id (allowed: [A-Za-z0-9._-])",
        );
    }

    let run_dir = run_bundle_dir(&state.data_dir, &id);
    if !fs::try_exists(&run_dir).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "run bundle not found");
    }

    let capture_dir = match resolve_run_bundle_capture_dir(&run_dir).await {
        Ok(p) => p,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("resolve capture dir failed: {e}"),
            );
        }
    };

    let mut rd = match fs::read_dir(&capture_dir).await {
        Ok(rd) => rd,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                return json_error(StatusCode::NOT_FOUND, "no captures yet");
            }
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read capture dir failed: {e}"),
            );
        }
    };

    let mut best: Option<(PathBuf, u128)> = None;
    while let Ok(Some(ent)) = rd.next_entry().await {
        let name = ent.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pcap") {
            continue;
        }
        let meta = match ent.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };
        let ms = match meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        {
            Some(d) => d.as_millis(),
            None => continue,
        };
        let path = ent.path();
        if best.as_ref().map(|(_, bms)| ms > *bms).unwrap_or(true) {
            best = Some((path, ms));
        }
    }

    let Some((path, _ms)) = best else {
        return json_error(StatusCode::NOT_FOUND, "no captures yet");
    };

    // Prevent accidental huge allocations.
    let meta = match fs::metadata(&path).await {
        Ok(m) => m,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("stat capture file failed: {e}"),
            );
        }
    };
    let max_bytes: u64 = 128 * 1024 * 1024;
    if meta.len() > max_bytes {
        return json_error(
            StatusCode::BAD_REQUEST,
            format!(
                "latest capture is too large to download via API ({} bytes; max {} bytes)",
                meta.len(),
                max_bytes
            ),
        );
    }

    let bytes = match fs::read(&path).await {
        Ok(b) => b,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read capture file failed: {e}"),
            );
        }
    };

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "capture.pcap".to_string());

    let mut resp = axum::response::Response::new(axum::body::Body::from(bytes));
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/vnd.tcpdump.pcap"),
    );
    resp.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        match axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
            Ok(v) => v,
            Err(_) => axum::http::HeaderValue::from_static("attachment"),
        },
    );
    resp
}

pub async fn view_run_bundle_output(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let id = id.trim().to_string();
    if !is_safe_id(&id) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid id (allowed: [A-Za-z0-9._-])",
        );
    }

    let run_dir = run_bundle_dir(&state.data_dir, &id);
    if !fs::try_exists(&run_dir).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "run bundle not found");
    }

    let output_dir = run_dir.join("output");
    let output_dir_abs: PathBuf = std::fs::canonicalize(&output_dir).unwrap_or(output_dir.clone());

    let logs_dir = output_dir.join("logs");
    let pcap_dir = output_dir.join("pcap");

    async fn list_names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut rd = match fs::read_dir(dir).await {
            Ok(r) => r,
            Err(_) => return out,
        };
        while let Ok(Some(ent)) = rd.next_entry().await {
            let name = ent.file_name().to_string_lossy().to_string();
            if is_safe_filename(&name) {
                out.push(name);
            }
        }
        out.sort();
        out
    }

    let log_files = list_names(&logs_dir).await;
    let pcap_files = list_names(&pcap_dir).await;

    let mut html = String::new();
    html.push_str("<!doctype html><html><head><meta charset=\"utf-8\"/>");
    html.push_str("<title>Ntx run-bundle output</title>");
    html.push_str("<style>body{font-family:system-ui,Arial,sans-serif;margin:16px} code,pre{font-family:ui-monospace,Menlo,monospace} a{color:#2563eb;text-decoration:none} a:hover{text-decoration:underline} .muted{color:#6b7280}</style>");
    html.push_str("</head><body>");

    html.push_str("<h2>Run bundle output</h2>");
    html.push_str(&format!(
        "<div class=\"muted\">id: <code>{}</code></div>",
        html_escape(&id)
    ));
    html.push_str(&format!(
        "<div class=\"muted\" style=\"margin-top:6px\">output dir: <code>{}</code></div>",
        html_escape(&output_dir_abs.display().to_string())
    ));

    html.push_str("<h3 style=\"margin-top:16px\">logs</h3>");
    if log_files.is_empty() {
        html.push_str("<div class=\"muted\">(empty)</div>");
    } else {
        html.push_str("<ul>");
        for name in log_files {
            html.push_str(&format!(
                "<li><a href=\"/api/v1/run-bundles/{}/output/files/logs/{}\">{}</a></li>",
                html_escape(&id),
                html_escape(&name),
                html_escape(&name)
            ));
        }
        html.push_str("</ul>");
    }

    html.push_str("<h3 style=\"margin-top:16px\">pcap</h3>");
    if pcap_files.is_empty() {
        html.push_str("<div class=\"muted\">(empty)</div>");
    } else {
        html.push_str("<ul>");
        for name in pcap_files {
            html.push_str(&format!(
                "<li><a href=\"/api/v1/run-bundles/{}/output/files/pcap/{}\">{}</a></li>",
                html_escape(&id),
                html_escape(&name),
                html_escape(&name)
            ));
        }
        html.push_str("</ul>");
    }

    html.push_str("</body></html>");

    (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

pub async fn download_run_bundle_output_file(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path((id, kind, name)): axum::extract::Path<(String, String, String)>,
) -> impl IntoResponse {
    let id = id.trim().to_string();
    if !is_safe_id(&id) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid id (allowed: [A-Za-z0-9._-])",
        );
    }

    let kind = kind.trim();
    if kind != "logs" && kind != "pcap" {
        return json_error(StatusCode::BAD_REQUEST, "invalid kind (expected logs|pcap)");
    }
    if !is_safe_filename(&name) {
        return json_error(StatusCode::BAD_REQUEST, "invalid filename");
    }

    let run_dir = run_bundle_dir(&state.data_dir, &id);
    if !fs::try_exists(&run_dir).await.unwrap_or(false) {
        return json_error(StatusCode::NOT_FOUND, "run bundle not found");
    }

    let path = run_dir.join("output").join(kind).join(&name);
    let meta = match fs::metadata(&path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return json_error(StatusCode::NOT_FOUND, "file not found");
        }
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("stat output file failed: {e}"),
            );
        }
    };

    let max_bytes: u64 = 256 * 1024 * 1024;
    if meta.len() > max_bytes {
        return json_error(
            StatusCode::BAD_REQUEST,
            format!(
                "file is too large to download via API ({} bytes; max {} bytes)",
                meta.len(),
                max_bytes
            ),
        );
    }

    let bytes = match fs::read(&path).await {
        Ok(b) => b,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read output file failed: {e}"),
            );
        }
    };

    let content_type = if kind == "pcap" || name.ends_with(".pcap") {
        "application/vnd.tcpdump.pcap"
    } else {
        "text/plain; charset=utf-8"
    };

    let mut resp = axum::response::Response::new(axum::body::Body::from(bytes));
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("application/octet-stream")),
    );
    resp.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        match axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{}\"", name)) {
            Ok(v) => v,
            Err(_) => axum::http::HeaderValue::from_static("attachment"),
        },
    );
    resp
}
