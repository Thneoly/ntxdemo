use std::sync::Arc;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::{routes, state::AppState};

pub const ROUTES: &[&str] = &[
    "GET /healthz",
    "GET /api/v1/routes",
    "GET /api/v1/config-bundles",
    "POST /api/v1/config-bundles",
    "GET /api/v1/config-bundles/{name}",
    "POST /api/v1/run-bundles",
    "POST /api/v1/run-bundles/{id}/run",
    "GET /api/v1/run-bundles/{id}/status",
    "POST /api/v1/run-bundles/{id}/stop",
    "GET /api/v1/run-bundles/{id}/logs",
    "GET /api/v1/catalog",
    "POST /api/v1/catalog",
    "POST /api/v1/ingest",
    "GET /api/v1/wasm",
    "POST /api/v1/wasm/upload",
    "GET /api/v1/wasm/{sha256}",
    "GET /api/v1/wasm/{sha256}/catalog",
    "POST /api/v1/wasm/push",
    "POST /api/v1/workflows",
    "GET /api/v1/workflows/{id}",
];

pub fn build_app(state: Arc<AppState>, cors_any_origin: bool) -> Router {
    let mut app = Router::new()
        .route("/healthz", get(routes::health::healthz))
        .route("/api/v1/routes", get(routes::meta::get_routes))
        .route(
            "/api/v1/config-bundles",
            get(routes::config_bundles::list_config_bundles)
                .post(routes::config_bundles::put_config_bundle),
        )
        .route(
            "/api/v1/config-bundles/{name}",
            get(routes::config_bundles::get_config_bundle),
        )
        .route(
            "/api/v1/run-bundles",
            post(routes::run_bundles::create_run_bundle),
        )
        .route(
            "/api/v1/run-bundles/{id}/run",
            post(routes::run_bundles::run_run_bundle),
        )
        .route(
            "/api/v1/run-bundles/{id}/status",
            get(routes::run_bundles::get_run_bundle_status),
        )
        .route(
            "/api/v1/run-bundles/{id}/stop",
            post(routes::run_bundles::stop_run_bundle),
        )
        .route(
            "/api/v1/run-bundles/{id}/logs",
            get(routes::run_bundles::get_run_bundle_logs),
        )
        .route(
            "/api/v1/catalog",
            get(routes::catalog::get_catalog).post(routes::catalog::put_catalog),
        )
        .route("/api/v1/ingest", post(routes::ingest::ingest))
        .route("/api/v1/wasm", get(routes::wasm::list_wasm))
        .route("/api/v1/wasm/upload", post(routes::wasm::upload_wasm))
        .route(
            "/api/v1/wasm/{sha256}/catalog",
            get(routes::wasm::get_wasm_catalog),
        )
        .route("/api/v1/wasm/{sha256}", get(routes::wasm::download_wasm))
        .route("/api/v1/wasm/push", post(routes::wasm::push_wasm))
        .route(
            "/api/v1/workflows",
            post(routes::workflows::create_workflow),
        )
        .route(
            "/api/v1/workflows/{id}",
            get(routes::workflows::get_workflow),
        )
        // Allow uploading larger wasm components (Axum `Multipart` uses this limit).
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(
                    DefaultMakeSpan::new()
                        .level(Level::INFO)
                        .include_headers(true),
                )
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(
                    DefaultOnResponse::new()
                        .level(Level::INFO)
                        .include_headers(true),
                )
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
        .with_state(state);

    if cors_any_origin {
        app = app.layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods(Any),
        );
    }

    app
}
