use std::{env, fs, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::{ConnectInfo, OriginalUri, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;

#[derive(Clone)]
struct AppState {
    listen_addr: SocketAddr,
    asset_meta: AssetMeta,
    asset_body: Value,
    asset_status: u16,
    health_payload: Value,
    json_payload: Value,
}

#[derive(Serialize, Clone)]
struct AssetResponse {
    ip: String,
    port: u16,
    status_code: u16,
    asset: AssetMeta,
}

#[derive(Serialize, Clone, Debug)]
struct AssetMeta {
    path: String,
    version: String,
}

#[derive(Debug, Clone)]
struct ServerOptions {
    listen_addr: SocketAddr,
    asset_meta: AssetMeta,
    asset_body: Value,
    asset_status: u16,
    health_payload: Value,
    json_payload: Value,
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    listen_addr: Option<String>,
    asset: Option<FileAssetConfig>,
    responses: Option<FileResponses>,
}

#[derive(Debug, Deserialize)]
struct FileAssetConfig {
    path: Option<String>,
    version: Option<String>,
    status_code: Option<u16>,
    body: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct FileResponses {
    json: Option<Value>,
    health: Option<Value>,
}

#[derive(Debug)]
struct CliOptions {
    listen_addr: Option<String>,
    config_path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let state = build_app_state()?;
    let addr = state.listen_addr;

    let app = Router::new()
        .route("/asset", get(handle_get_asset).post(handle_post_asset))
        .route("/get", get(handle_get_asset))
        .route("/post", post(handle_post_asset))
        .route("/health", get(handle_health))
        .route("/json", get(handle_get_json))
        .with_state(state.clone());

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind to {}", addr))?;
    println!("HTTP test server listening on http://{}", addr);

    let service = app.into_make_service_with_connect_info::<SocketAddr>();

    axum::serve(listener, service)
        .await
        .context("server terminated unexpectedly")?;

    Ok(())
}

fn build_app_state() -> Result<AppState> {
    let cli = parse_cli_options()?;
    let mut options = ServerOptions::default();

    if let Some(config_path) = cli
        .config_path
        .or_else(|| env::var("HTTP_SERVER_CONFIG").ok().map(PathBuf::from))
    {
        let cfg = load_file_config(&config_path)?;
        options.apply(cfg)?;
        println!("Loaded HTTP server config from {}", config_path.display());
    }

    if let Some(addr) = cli.listen_addr.or_else(|| env::var("HTTP_TEST_ADDR").ok()) {
        options.listen_addr = addr
            .parse()
            .with_context(|| format!("invalid socket address `{}`", addr))?;
    }

    Ok(options.into())
}

fn parse_cli_options() -> Result<CliOptions> {
    let mut args = env::args().skip(1);
    let mut listen_addr = None;
    let mut config_path = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => {
                let path = args
                    .next()
                    .ok_or_else(|| anyhow!("`--config` requires a file path"))?;
                config_path = Some(PathBuf::from(path));
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ => {
                if listen_addr.is_none() {
                    listen_addr = Some(arg);
                } else {
                    return Err(anyhow!("unexpected argument `{}`", arg));
                }
            }
        }
    }

    Ok(CliOptions {
        listen_addr,
        config_path,
    })
}

fn print_help() {
    eprintln!(
        "Usage: http_server [ADDR] [--config <file>]\n\nOptions:\n    -c, --config <file>    Load response defaults from YAML/JSON file\n    -h, --help             Show this message\n\nEnv vars:\n    HTTP_TEST_ADDR         Override listen address (e.g. 0.0.0.0:9000)\n    HTTP_SERVER_CONFIG     Path to the same config file as --config\n"
    );
}

async fn handle_get_asset(
    OriginalUri(uri): OriginalUri,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> Json<AssetResponse> {
    log_request("GET", uri.path(), &remote, None);

    let response = AssetResponse {
        ip: state.listen_addr.ip().to_string(),
        port: state.listen_addr.port(),
        status_code: state.asset_status,
        asset: state.asset_meta.clone(),
    };

    log_asset_response("GET", uri.path(), &remote, &response);
    Json(response)
}

async fn handle_health(
    OriginalUri(uri): OriginalUri,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> Json<Value> {
    log_request("GET", uri.path(), &remote, None);

    let body = json!({
        "ip": state.listen_addr.ip().to_string(),
        "port": state.listen_addr.port(),
        "status_code": state.asset_status,
        "payload": state.health_payload.clone(),
    });

    log_response("GET", uri.path(), &remote, state.asset_status, &body);
    Json(body)
}

async fn handle_get_json(
    OriginalUri(uri): OriginalUri,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> Json<Value> {
    log_request("GET", uri.path(), &remote, None);

    let body = json!({
        "ip": state.listen_addr.ip().to_string(),
        "port": state.listen_addr.port(),
        "status_code": state.asset_status,
        "payload": state.json_payload.clone(),
    });

    log_response("GET", uri.path(), &remote, state.asset_status, &body);
    Json(body)
}

async fn handle_post_asset(
    OriginalUri(uri): OriginalUri,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    log_request("POST", uri.path(), &remote, Some(&payload));

    let body = json!({
        "ip": state.listen_addr.ip().to_string(),
        "port": state.listen_addr.port(),
        "status_code": state.asset_status,
        "result": payload,
        "expected_asset": state.asset_body.clone(),
    });

    log_response("POST", uri.path(), &remote, state.asset_status, &body);
    Json(body)
}

fn load_file_config(path: &PathBuf) -> Result<FileConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config `{}`", path.display()))?;
    let cfg: FileConfig = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse config `{}`", path.display()))?;
    Ok(cfg)
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".parse().expect("valid addr"),
            asset_meta: AssetMeta {
                path: "/asset".to_string(),
                version: "v1".to_string(),
            },
            asset_body: json!({
                "id": "local-asset",
                "name": "demo",
                "type": "example",
            }),
            asset_status: 200,
            health_payload: json!({
                "status": "ok",
                "version": "v1",
            }),
            json_payload: json!({
                "message": "hello from demo server",
                "version": "v1",
            }),
        }
    }
}

impl ServerOptions {
    fn apply(&mut self, cfg: FileConfig) -> Result<()> {
        if let Some(addr) = cfg.listen_addr {
            self.listen_addr = addr
                .parse()
                .with_context(|| format!("invalid socket address `{}`", addr))?;
        }

        if let Some(asset_cfg) = cfg.asset {
            if let Some(path) = asset_cfg.path {
                self.asset_meta.path = path;
            }
            if let Some(version) = asset_cfg.version {
                self.asset_meta.version = version;
            }
            if let Some(status) = asset_cfg.status_code {
                self.asset_status = status;
            }
            if let Some(body) = asset_cfg.body {
                self.asset_body = body;
            }
        }

        if let Some(responses) = cfg.responses {
            if let Some(json_payload) = responses.json {
                self.json_payload = json_payload;
            }
            if let Some(health_payload) = responses.health {
                self.health_payload = health_payload;
            }
        }

        Ok(())
    }
}

impl From<ServerOptions> for AppState {
    fn from(options: ServerOptions) -> Self {
        Self {
            listen_addr: options.listen_addr,
            asset_meta: options.asset_meta,
            asset_body: options.asset_body,
            asset_status: options.asset_status,
            health_payload: options.health_payload,
            json_payload: options.json_payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::ConnectInfo, http::Uri};

    #[tokio::test]
    async fn get_asset_returns_meta() {
        let state = AppState {
            listen_addr: "10.0.0.1:7000".parse().unwrap(),
            asset_meta: AssetMeta {
                path: "/asset".into(),
                version: "v1".into(),
            },
            asset_body: json!({"id": 1}),
            asset_status: 200,
            health_payload: json!({"status": "ok"}),
            json_payload: json!({"message": "hello"}),
        };

        let Json(body) = handle_get_asset(
            OriginalUri(Uri::from_static("/asset")),
            ConnectInfo("203.0.113.10:4000".parse().unwrap()),
            axum::extract::State(state.clone()),
        )
        .await;

        assert_eq!(body.ip, state.listen_addr.ip().to_string());
        assert_eq!(body.port, state.listen_addr.port());
        assert_eq!(body.status_code, 200);
        assert_eq!(body.asset.path, "/asset");
        assert_eq!(body.asset.version, "v1");
    }

    #[tokio::test]
    async fn post_asset_echoes_payload() {
        let state = AppState {
            listen_addr: "192.168.1.2:8081".parse().unwrap(),
            asset_meta: AssetMeta {
                path: "/asset".into(),
                version: "v1".into(),
            },
            asset_body: json!({"expected": true}),
            asset_status: 201,
            health_payload: json!({"status": "ok"}),
            json_payload: json!({"message": "hello"}),
        };
        let payload = json!({
            "asset": {
                "path": "/asset",
                "body": {"value": 42},
            }
        });

        let Json(body) = handle_post_asset(
            OriginalUri(Uri::from_static("/asset")),
            ConnectInfo("203.0.113.11:5000".parse().unwrap()),
            axum::extract::State(state.clone()),
            Json(payload.clone()),
        )
        .await;

        assert_eq!(body["ip"], state.listen_addr.ip().to_string());
        assert_eq!(body["port"], state.listen_addr.port());
        assert_eq!(body["status_code"], 201);
        assert_eq!(body["result"], payload);
    }
}

fn log_request(method: &str, path: &str, remote: &SocketAddr, payload: Option<&Value>) {
    if let Some(body) = payload {
        println!(
            "[http_server] <= {} {} from={} payload={}",
            method,
            path,
            remote,
            serde_json::to_string(body).unwrap_or_else(|_| "<unprintable>".into())
        );
    } else {
        println!("[http_server] <= {} {} from={}", method, path, remote);
    }
}

fn log_response(method: &str, path: &str, remote: &SocketAddr, status: u16, body: &Value) {
    println!(
        "[http_server] => {} {} to={} status={} body={}",
        method,
        path,
        remote,
        status,
        serde_json::to_string(body).unwrap_or_else(|_| "<unprintable>".into())
    );
}

fn log_asset_response(method: &str, path: &str, remote: &SocketAddr, response: &AssetResponse) {
    match serde_json::to_value(response) {
        Ok(value) => log_response(method, path, remote, response.status_code, &value),
        Err(_) => println!(
            "[http_server] => {} {} to={} status={} body=<serialize error>",
            method, path, remote, response.status_code
        ),
    }
}
