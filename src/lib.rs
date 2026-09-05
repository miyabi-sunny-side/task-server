#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]
use axum::{
    Router,
    routing::{get, post},
};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
pub mod clock;
pub mod error;
pub mod frontmatter;
pub mod http;
pub mod ledger;
pub mod mcp;
pub mod product;
pub mod report;
pub mod runs;
pub mod scan;
pub mod state;
pub mod task;
pub use clock::{Clock, SharedClock, SystemClock, format_z};
pub use error::Error;
pub use state::AppState;
pub fn app(state: AppState) -> Router {
    let static_dir = state.static_dir.clone();
    let api = Router::new()
        .route("/health", get(http::api_health))
        .route("/session", get(http::api_session))
        .route("/tasks", get(http::api_tasks).post(http::api_create_task))
        .route(
            "/tasks/{id}",
            get(http::api_task)
                .patch(http::api_patch_task)
                .delete(http::api_delete_task),
        )
        .route("/tasks/{id}/status", post(http::api_set_status))
        .route("/done", get(http::api_done))
        .route("/closed", get(http::api_closed))
        .route("/control", get(http::api_control))
        .route("/runs", get(http::api_runs).post(http::api_runs_post))
        .route("/runs/next", get(http::api_runs_next))
        .route("/runs/{id}", get(http::api_run))
        .route("/runs/{id}/read", post(http::api_run_read))
        .route("/products", get(http::api_products))
        .route("/products/rescan", post(http::api_rescan_products))
        .route(
            "/products/{*id}",
            get(http::api_product).put(http::api_put_product),
        )
        .route("/merges", post(http::retired))
        .route("/reviews", post(http::retired))
        .route("/releases", post(http::retired))
        .fallback(http::api_not_found);
    Router::new()
        .route("/healthz", get(http::healthz))
        .nest("/api", api)
        .route("/worker/claim", post(http::worker_claim))
        .route("/worker/heartbeat", post(http::worker_heartbeat))
        .route("/worker/report", post(http::worker_report))
        .route("/worker/runs", post(http::worker_runs))
        .route("/worker/snapshot", get(http::worker_snapshot))
        .route("/worker/claim/release", post(http::retired))
        .route("/worker/review-report", post(http::retired))
        .merge(mcp::endpoints(&state))
        .fallback_service(
            ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html"))),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
