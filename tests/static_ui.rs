use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use task_server::{AppState, ledger::Store};
use tower::ServiceExt;

async fn request(app: axum::Router, method: &str, path: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn embedded_ui_serves_html_assets_and_client_routes() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(Store::open(dir.path().join("ledger")).unwrap());
    let app = task_server::app(state);
    let response = request(app.clone(), "GET", "/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/html");
    let html = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(html.starts_with(b"<!doctype html>"));

    let document = std::str::from_utf8(&html).unwrap();
    for (extension, mime) in [(".js", "text/javascript"), (".css", "text/css")] {
        let path = document
            .split('"')
            .find(|value| value.starts_with("/assets/") && value.ends_with(extension))
            .expect("built HTML references an asset");
        let response = request(app.clone(), "GET", &format!("{path}?v=1")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], mime);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(!body.is_empty());
        assert_ne!(body, html);
        let head = request(app.clone(), "HEAD", path).await;
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()["content-type"], mime);
        assert!(
            to_bytes(head.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );
    }

    for path in [
        "/tasks/example",
        "/closed",
        "/products",
        "/projects/example",
    ] {
        let response = request(app.clone(), "GET", path).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            html
        );
    }
    assert_eq!(
        request(app, "POST", "/").await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn api_worker_and_mcp_routes_keep_their_responses() {
    let dir = tempfile::tempdir().unwrap();
    let app = task_server::app(AppState::new(Store::open(dir.path()).unwrap()));
    for path in ["/api", "/api/", "/api/missing"] {
        let response = request(app.clone(), "GET", path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(response.headers()["content-type"], "application/json");
    }
    for path in ["/api/health", "/worker/snapshot"] {
        let response = request(app.clone(), "GET", path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()["content-type"], "application/json");
    }
    for path in ["/mcp", "/worker/mcp"] {
        let response = request(app.clone(), "GET", path).await;
        assert_ne!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/html")
        );
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }
}
