use std::{env, net::SocketAddr};

use anyhow::{Context, Result};
use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;

#[derive(Clone)]
struct AppState {
    ip: String,
    port: u16,
}

#[derive(Serialize)]
struct AssetResponse {
    ip: String,
    port: u16,
    status_code: u16,
    asset: AssetMeta,
}

#[derive(Serialize)]
struct AssetMeta {
    path: String,
    version: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let addr = resolve_addr()?;
    let state = AppState {
        ip: addr.ip().to_string(),
        port: addr.port(),
    };

    let app = Router::new()
        .route("/asset", get(handle_get_asset).post(handle_post_asset))
        .with_state(state.clone());

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind to {}", addr))?;
    println!("HTTP test server listening on http://{}", addr);

    axum::serve(listener, app)
        .await
        .context("server terminated unexpectedly")?;

    Ok(())
}

fn resolve_addr() -> Result<SocketAddr> {
    if let Some(addr) = env::args().nth(1) {
        return addr
            .parse()
            .with_context(|| format!("invalid socket address `{}`", addr));
    }

    if let Ok(addr) = env::var("HTTP_TEST_ADDR") {
        return addr
            .parse()
            .with_context(|| format!("invalid socket address `{}`", addr));
    }

    Ok("127.0.0.1:8080".parse().expect("valid default addr"))
}

async fn handle_get_asset(State(state): State<AppState>) -> Json<AssetResponse> {
    Json(AssetResponse {
        ip: state.ip,
        port: state.port,
        status_code: 200,
        asset: AssetMeta {
            path: "/asset".to_string(),
            version: "v1".to_string(),
        },
    })
}

async fn handle_post_asset(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    Json(json!({
        "ip": state.ip,
        "port": state.port,
        "status_code": 200,
        "result": payload,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_asset_returns_meta() {
        let state = AppState {
            ip: "10.0.0.1".to_string(),
            port: 7000,
        };

        let Json(body) = handle_get_asset(axum::extract::State(state.clone())).await;

        assert_eq!(body.ip, state.ip);
        assert_eq!(body.port, state.port);
        assert_eq!(body.status_code, 200);
        assert_eq!(body.asset.path, "/asset");
        assert_eq!(body.asset.version, "v1");
    }

    #[tokio::test]
    async fn post_asset_echoes_payload() {
        let state = AppState {
            ip: "192.168.1.2".to_string(),
            port: 8081,
        };
        let payload = json!({
            "asset": {
                "path": "/asset",
                "body": {"value": 42},
            }
        });

        let Json(body) =
            handle_post_asset(axum::extract::State(state.clone()), Json(payload.clone())).await;

        assert_eq!(body["ip"], state.ip);
        assert_eq!(body["port"], state.port);
        assert_eq!(body["status_code"], 200);
        assert_eq!(body["result"], payload);
    }
}
