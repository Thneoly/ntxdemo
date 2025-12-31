use std::sync::Arc;

use axum::{Router, routing::{get, post}};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::{
    routes,
    state::AppState,
};

pub fn build_app(state: Arc<AppState>, cors_any_origin: bool) -> Router {
    let mut app = Router::new()
        .route("/healthz", get(routes::health::healthz))
        .route(
            "/api/v1/catalog",
            get(routes::catalog::get_catalog).post(routes::catalog::put_catalog),
        )
        .route("/api/v1/ingest", post(routes::ingest::ingest))
        .route("/api/v1/wasm", get(routes::wasm::list_wasm))
        .route("/api/v1/wasm/upload", post(routes::wasm::upload_wasm))
        .route("/api/v1/wasm/{sha256}", get(routes::wasm::download_wasm))
        .route("/api/v1/wasm/push", post(routes::wasm::push_wasm))
        .route("/api/v1/workflows", post(routes::workflows::create_workflow))
        .route(
            "/api/v1/workflows/{id}",
            get(routes::workflows::get_workflow),
        )
        .layer(TraceLayer::new_for_http())
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
