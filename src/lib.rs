#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]
use axum::{
    Router,
    http::{Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tower_http::trace::TraceLayer;

static UI: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/client/dist");
pub mod checkpoint;
pub mod clock;
pub mod error;
pub mod frontmatter;
pub mod http;
pub mod ledger;
pub mod mcp;
pub mod product;
pub mod report;
pub mod runs;
pub mod state;
pub mod task;
pub use clock::{Clock, SharedClock, SystemClock, format_z};
pub use error::Error;
pub use state::AppState;
pub fn app(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(http::api_health))
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
        .route("/products/rescan", post(http::retired))
        .route(
            "/products/{*id}",
            get(http::api_product)
                .put(http::api_put_product)
                .patch(http::api_patch_product),
        )
        .route("/merges", post(http::retired))
        .route("/reviews", post(http::retired))
        .route("/releases", post(http::retired))
        .fallback(http::api_not_found);
    Router::new()
        .route("/healthz", get(http::healthz))
        .route("/api", axum::routing::any(http::api_not_found))
        .route("/api/", axum::routing::any(http::api_not_found))
        .nest("/api", api)
        .route("/worker/products/{*id}", get(http::worker_product))
        .route("/worker/claim", post(http::worker_claim))
        .route("/worker/heartbeat", post(http::worker_heartbeat))
        .route(
            "/worker/tasks/{id}/checkpoint",
            get(http::worker_checkpoint).post(http::worker_checkpoint_update),
        )
        .route("/worker/report", post(http::worker_report))
        .route("/worker/runs", post(http::worker_runs))
        .route("/worker/snapshot", get(http::worker_snapshot))
        .route("/worker/claim/release", post(http::retired))
        .route("/worker/review-report", post(http::retired))
        .merge(mcp::endpoints(&state))
        .fallback_service(get(ui))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn ui(uri: Uri) -> Response {
    let file = UI
        .get_file(uri.path().trim_start_matches('/'))
        .unwrap_or_else(|| {
            UI.get_file("index.html")
                .expect("build requires index.html")
        });
    let content_type = mime_guess::from_path(file.path()).first_or_octet_stream();
    (
        [(header::CONTENT_TYPE, content_type.as_ref())],
        file.contents(),
    )
        .into_response()
}
