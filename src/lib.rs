//! Task Server — household task control plane.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::path::Path;

use axum::Router;
use axum::routing::{get, post};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

pub mod clock;
pub mod db;
pub mod error;
pub mod frontmatter;
pub mod http;
pub mod import;
pub mod mcp;
pub mod product;
pub mod scan;
pub mod state;
pub mod task;

pub use clock::{Clock, SharedClock, SystemClock, format_z};
pub use db::Db;
pub use error::Error;
pub use frontmatter::{Document, join_document, split_document};
pub use http::{ControlPlane, DoneSummary, PendingMerge, PendingRelease, TaskCard, TaskSummary};
pub use import::{ImportError, ImportSources, ImportSummary, import_markdown};
pub use product::Product;
pub use state::AppState;
pub use task::{
    Check, NewTask, Releasable, ReportOutcome, ReviewOutcome, ReviewVerdict, Task, TaskKind,
    TaskPatch, TaskStatus, can_transition,
};

pub fn app(state: AppState) -> Router {
    let static_dir = state.static_dir.clone();
    let api = Router::new()
        .route("/health", get(http::api_health))
        .route("/session", get(http::api_session))
        .route("/tasks", get(http::api_tasks).post(http::api_create_task))
        .route("/done", get(http::api_done))
        .route(
            "/tasks/{id}",
            get(http::api_task).patch(http::api_patch_task),
        )
        .route("/tasks/{id}/status", post(http::api_set_status))
        .route("/control", get(http::api_control))
        .route("/merges", post(http::api_issue_merge))
        .route("/reviews", post(http::api_issue_review))
        .route("/releases", post(http::api_release))
        .route("/products", get(http::api_products))
        // Product ids are `org/repo`, so the capture has to span two segments.
        .route(
            "/products/{*id}",
            get(http::api_product).put(http::api_put_product),
        )
        .fallback(http::api_not_found);

    Router::new()
        .route("/healthz", get(http::healthz))
        .route("/worker/claim", post(http::worker_claim))
        .route("/worker/claim/release", post(http::worker_claim_release))
        .route("/worker/report", post(http::worker_report))
        .route("/worker/review-report", post(http::worker_review_report))
        // Same domain layer, second transport. Both MCP faces share the worker
        // HTTP routes' trusted-network boundary.
        .merge(mcp::endpoints(&state))
        .nest("/api", api)
        .fallback_service(
            ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html"))),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Convenience for tests that only need a static file root.
pub fn app_with_static(static_dir: impl AsRef<Path>) -> Router {
    app(AppState::for_test().with_static_dir(static_dir.as_ref()))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::app_with_static;

    #[tokio::test]
    async fn liveness_is_lightweight_plain_text() {
        let response = app_with_static("client/dist")
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            "ok\n"
        );
    }

    #[tokio::test]
    async fn api_health_returns_stable_json() {
        let response = app_with_static("client/dist")
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            r#"{"status":"ok"}"#
        );
    }

    #[tokio::test]
    async fn unknown_api_routes_do_not_fall_back_to_the_spa() {
        let response = app_with_static("client/dist")
            .oneshot(
                Request::builder()
                    .uri("/api/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_client_routes_return_the_spa_with_success() {
        let response = app_with_static("client")
            .oneshot(
                Request::builder()
                    .uri("/projects/example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("content-type").unwrap(), "text/html");
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.starts_with(b"<!doctype html>"));
    }
}
