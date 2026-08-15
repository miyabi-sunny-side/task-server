//! Task Server — household task control plane.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::path::Path;

use axum::Router;
use axum::routing::{get, post};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

pub mod actions;
pub mod clock;
pub mod db;
pub mod error;
pub mod frontmatter;
pub mod http;
pub mod notify;
pub mod outbox;
pub mod product;
pub mod state;
pub mod status;
pub mod store;
pub mod task;

pub use actions::{ActionEffect, ActionTable};
pub use clock::{Clock, SharedClock, SystemClock, format_z};
pub use db::Db;
pub use error::Error;
pub use frontmatter::{Document, join_document, split_document};
pub use notify::{FailingNotifier, HttpNotifier, NoopNotifier, Notifier, flush_pending};
pub use outbox::NotificationIntent;
pub use product::Product;
pub use state::AppState;
pub use status::{Status, TransitionContext, can_transition, validate_task};
pub use store::{
    ClaimLease, ReportOutcome, ReportRequest, TaskCard, TaskSummary, apply_human_action, claim,
    get_task, list_tasks, report, self_service_awaiting_user,
};
pub use task::{NewTask, Task, TaskKind, TaskStatus};

pub fn app(state: AppState) -> Router {
    let static_dir = state.static_dir.clone();
    let api = Router::new()
        .route("/health", get(http::api_health))
        .route("/session", get(http::api_session))
        .route("/tasks", get(http::api_tasks))
        .route("/tasks/{id}", get(http::api_task))
        .route("/tasks/{id}/actions/{action}", post(http::api_action))
        .fallback(http::api_not_found);

    Router::new()
        .route("/healthz", get(http::healthz))
        .route("/worker/claim", post(http::worker_claim))
        .route("/worker/report", post(http::worker_report))
        .nest("/api", api)
        .fallback_service(
            ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html"))),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Convenience for tests that only need a static file root.
pub fn app_with_static(static_dir: impl AsRef<Path>) -> Router {
    app(AppState::for_test("/nonexistent-tasks-git-dir").with_static_dir(static_dir.as_ref()))
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
