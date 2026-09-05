use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;
async fn request(
    app: axum::Router,
    method: &str,
    path: &str,
    body: serde_json::Value,
    auth: bool,
    csrf: bool,
) -> (StatusCode, serde_json::Value) {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if auth {
        r = r.header("x-auth-user", "test");
    }
    if csrf {
        r = r.header("x-csrf-token", "test-csrf");
    }
    let r = app
        .oneshot(r.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}
use task_server::{AppState, SharedClock, ledger::Store, task};
use time::macros::datetime;
#[test]
fn lease_interruption_and_resume_preserve_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let clock = SharedClock::at(datetime!(2026-09-05 00:00 UTC));
    let state = AppState::new(Store::open(dir.path()).unwrap())
        .with_clock(Arc::new(clock.clone()))
        .with_ttl(10);
    state
        .store
        .put(
            "products",
            "a/b",
            json!({"id":"a/b","repository":"https://example/a/b","archived":false}),
        )
        .unwrap();
    task::create(&state, json!({"id":"t","title":"test","product_id":"a/b"})).unwrap();
    task::set_status(&state, "t", "ready").unwrap();
    let claim = task::claim(&state, "worker").unwrap().unwrap();
    assert!(task::claim(&state, "other").unwrap().is_none());
    clock.advance_secs(11);
    assert!(task::claim(&state, "other").unwrap().is_none());
    assert_eq!(state.store.get("tasks", "t").unwrap()["status"], "blocked");
    assert!(
        task::report(
            &state,
            json!({"claim_id":claim["claim_id"],"outcome":"done"})
        )
        .is_err()
    );
    task::set_status(&state, "t", "ready").unwrap();
    let c = task::claim(&state, "worker").unwrap().unwrap();
    task::report(&state,json!({"claim_id":c["claim_id"],"outcome":"done","commit_sha":"abc","milestones":[{"name":"verified","commit_sha":"abc","evidence":"cargo test passed"}]})).unwrap();
    task::patch(&state, "t", json!({"commit_sha":"def"})).unwrap();
    let t = state.store.get("tasks", "t").unwrap();
    assert_eq!(t["milestones"].as_array().unwrap().len(), 0);
    assert_eq!(t["milestone_history"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn http_auth_retirement_snapshot_and_run_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(Store::open(dir.path()).unwrap());
    let app = task_server::app(state.clone());

    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/api/tasks",
            json!({"title":"new"}),
            false,
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/api/tasks",
            json!({"title":"new"}),
            true,
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (code, t) = request(
        app.clone(),
        "POST",
        "/api/tasks",
        json!({"title":"new"}),
        true,
        true,
    )
    .await;
    assert_eq!(code, StatusCode::CREATED);
    assert!(t["id"].is_string());
    assert_eq!(
        request(app.clone(), "POST", "/api/merges", json!({}), true, true)
            .await
            .0,
        StatusCode::GONE
    );
    let run =
        json!({"source":"worker","claim_id":"c","attempt":1,"task_id":t["id"],"note":"evidence"});
    let (_, a) = request(
        app.clone(),
        "POST",
        "/worker/runs",
        run.clone(),
        false,
        false,
    )
    .await;
    let (_, b) = request(app.clone(), "POST", "/worker/runs", run, false, false).await;
    assert_eq!(a["id"], b["id"]);
    let path = format!("/api/runs/{}/read", a["id"]);
    let (code, r) = request(
        app.clone(),
        "POST",
        &path,
        json!({"note":"filed"}),
        true,
        false,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(r["read_note"], "filed");
    let (_, snap) = request(app, "GET", "/worker/snapshot", json!({}), false, false).await;
    assert_eq!(snap["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(snap["runs"].as_array().unwrap().len(), 1);
}

#[test]
fn report_resend_is_idempotent_but_conflicting_outcome_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::new(Store::open(dir.path()).unwrap());
    s.store
        .put("products", "a/b", json!({"id":"a/b","repository":"x"}))
        .unwrap();
    task::create(&s, json!({"id":"t","title":"test","product_id":"a/b"})).unwrap();
    task::set_status(&s, "t", "ready").unwrap();
    let c = task::claim(&s, "w").unwrap().unwrap();
    let r = json!({"claim_id":c["claim_id"],"outcome":"done","summary":"finished"});
    let first = task::report(&s, r.clone()).unwrap();
    assert_eq!(task::report(&s, r).unwrap(), first);
    assert!(task::report(&s, json!({"claim_id":c["claim_id"],"outcome":"blocked"})).is_err());
}

use axum::http::HeaderMap;
async fn rpc(
    app: axum::Router,
    session: Option<&str>,
    message: serde_json::Value,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    let response = app
        .oneshot(request.body(Body::from(message.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(status.is_success(), "MCP {status}: {text}");
    let value = serde_json::from_str(&text).unwrap_or_else(|_| {
        text.lines()
            .find_map(|line| {
                line.strip_prefix("data: ")
                    .and_then(|json| serde_json::from_str(json).ok())
            })
            .unwrap_or_default()
    });
    (status, headers, value)
}

#[tokio::test]
async fn mcp_flat_crud_contract_over_json_rpc() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(Store::open(dir.path()).unwrap());
    let app = task_server::app(state.clone());
    let (code, headers, initialized)=rpc(app.clone(),None,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"contract-test","version":"1"}}})).await;
    assert_eq!(code, StatusCode::OK);
    assert!(initialized["result"]["capabilities"]["tools"].is_object());
    let session = headers.get("mcp-session-id").unwrap().to_str().unwrap();
    rpc(
        app.clone(),
        Some(session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let (_, _, listed) = rpc(
        app.clone(),
        Some(session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    assert!(
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "task_create")
    );
    let (_,_,created)=rpc(app.clone(),Some(session),json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"task_create","arguments":{"id":"mcp-task","title":"flat arguments","body":"markdown"}}})).await;
    assert_eq!(
        created["result"]["structuredContent"]["title"],
        "flat arguments"
    );
    let (_,_,updated)=rpc(app.clone(),Some(session),json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"task_update","arguments":{"id":"mcp-task","title":"updated"}}})).await;
    assert_eq!(updated["result"]["structuredContent"]["title"], "updated");
    let (_,_,got)=rpc(app.clone(),Some(session),json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"task_get","arguments":{"id":"mcp-task"}}})).await;
    assert_eq!(got["result"]["structuredContent"]["body"], "markdown");
    state
        .store
        .put("products", "a/b", json!({"id":"a/b"}))
        .unwrap();
    task::patch(&state, "mcp-task", json!({"product_id":"a/b"})).unwrap();
    task::set_status(&state, "mcp-task", "ready").unwrap();
    let claim = task::claim(&state, "test").unwrap().unwrap();
    let payload = json!({"claim_id":claim["claim_id"],"outcome":"done","report_markdown":"# Original\nUnverified idea.","commit_sha":"abc","checks":[{"name":"cargo test","exit_code":0}],"milestones":[{"name":"implemented"}]});
    let (code, reported) = request(
        app.clone(),
        "POST",
        "/worker/report",
        payload.clone(),
        false,
        false,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let id = reported["report_id"].as_u64().unwrap().to_string();
    let (_,_,run)=rpc(app.clone(),Some(session),json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"run_get","arguments":{"id":id}}})).await;
    assert_eq!(
        run["result"]["structuredContent"]["body"],
        payload["report_markdown"]
    );
    assert_eq!(
        run["result"]["structuredContent"]["claim_id"],
        claim["claim_id"]
    );
    let (code, original) = request(
        app.clone(),
        "GET",
        &format!("/api/runs/{id}"),
        json!(null),
        true,
        false,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(original["body"], payload["report_markdown"]);
    assert_eq!(original["checks"][0]["exit_code"], 0);
    let (_, repeated) = request(app, "POST", "/worker/report", payload, false, false).await;
    assert_eq!(repeated["report_id"], reported["report_id"]);
    assert_eq!(state.store.list("runs").unwrap().len(), 1);
}

#[test]
fn rescan_preserves_migrated_product_documents_and_revives_archive() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let checkout = root.path().join("org/repo");
    gix::init(&checkout).unwrap();
    std::fs::write(
        checkout.join(".git/config"),
        "[core]\n bare = false\n[remote \"origin\"]\n url = https://example.com/org/repo\n",
    )
    .unwrap();
    let mut s = AppState::new(Store::open(data.path()).unwrap());
    s.projects_dir = Some(root.path().into());
    let original = json!({"id":"org/repo","repository":"old","description":"old","releases":true,"archived":true,"archived_at":"2025-01-01","created_at":"created","updated_at":"updated","body":"migration body","legacy":{"sqlite_columns":{"extra":"preserve"}},"custom":{"nested":[1,2]}});
    s.store
        .put("products", "org/repo", original.clone())
        .unwrap();
    for _ in 0..2 {
        task_server::product::rescan(&s).unwrap();
    }
    let p = s.store.get("products", "org/repo").unwrap();
    for key in ["legacy", "custom", "created_at", "updated_at", "body"] {
        assert_eq!(p[key], original[key], "{key}");
    }
    assert_eq!(p["archived"], false);
    assert!(p["archived_at"].is_null());
    assert_eq!(p["releases"], true);
}

#[test]
fn legacy_database_configuration_requires_explicit_migration() {
    assert!(
        matches!(AppState::from_vars(|key| (key=="APP_DB_PATH").then(||"old.db".into())),Err(task_server::Error::Invalid(message)) if message.contains("migrate"))
    );
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("task-server.db"), "old database").unwrap();
    let ledger = root.path().join("ledger");
    assert!(
        matches!(AppState::from_vars(|key| (key=="APP_DATA_DIR").then(||ledger.to_string_lossy().into_owned())),Err(task_server::Error::Invalid(message)) if message.contains("import-sqlite"))
    );
    assert!(!ledger.exists());
}

#[test]
fn claim_marks_missing_product_and_dependency_as_visible_blocking() {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::new(Store::open(dir.path()).unwrap());
    s.store
        .put("products", "a/b", json!({"id":"a/b","repository":"url"}))
        .unwrap();
    for (id, product, dependency) in [
        ("missing-product", "absent/repo", None),
        ("missing-dependency", "a/b", Some("gone")),
    ] {
        s.store.put("tasks",id,json!({"id":id,"status":"ready","kind":"normal","product_id":product,"depends_on":dependency})).unwrap();
    }
    assert!(task::claim(&s, "worker").unwrap().is_none());
    for id in ["missing-product", "missing-dependency"] {
        let t = s.store.get("tasks", id).unwrap();
        assert_eq!(t["status"], "blocked");
        assert!(!t["verification"].as_str().unwrap().is_empty());
    }
}

#[tokio::test]
async fn list_shapes_summary_projection_and_haystack_cursor_contract() {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::new(Store::open(dir.path()).unwrap());
    s.store.put("tasks","draft",json!({"id":"draft","title":"draft","status":"draft","kind":"normal","body":"large body","legacy":{"huge":"record"},"last_report":{"huge":"record"}})).unwrap();
    s.store.put("tasks","done",json!({"id":"done","title":"done","status":"done","kind":"normal","milestones":[],"closed_at":"2026","summary":"finished","legacy":{}})).unwrap();
    s.store
        .put(
            "products",
            "a/b",
            json!({"id":"a/b","repository":"url","legacy":{}}),
        )
        .unwrap();
    let app = task_server::app(s.clone());
    for path in ["/api/tasks", "/api/done", "/api/closed", "/api/products"] {
        let (status, list) = request(app.clone(), "GET", path, json!({}), true, false).await;
        assert_eq!(status, StatusCode::OK);
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].get("legacy").is_none());
        assert!(list[0].get("body").is_none());
        assert!(list[0].get("last_report").is_none());
    }
    let (_, detail) = request(
        app.clone(),
        "GET",
        "/api/tasks/draft",
        json!({}),
        true,
        false,
    )
    .await;
    assert_eq!(detail["body"], "large body");
    assert!(detail.get("legacy").is_some());
    for attempt in [1, 2] {
        task_server::runs::append(
            &s,
            json!({"source":"worker","claim_id":"claim","attempt":attempt}),
            false,
        )
        .unwrap();
    }
    let (_, first) = request(
        app.clone(),
        "GET",
        "/api/runs?limit=1",
        json!({}),
        true,
        false,
    )
    .await;
    assert_eq!(first["next"], 1);
    let (_, last) = request(
        app.clone(),
        "GET",
        "/api/runs?since=1&limit=1",
        json!({}),
        true,
        false,
    )
    .await;
    assert!(last["next"].is_null());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs/1/read")
                .header("x-auth-user", "test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (_, unread) = request(app, "GET", "/api/runs?unread=true", json!({}), true, false).await;
    assert_eq!(unread["runs"].as_array().unwrap().len(), 1);
    assert_eq!(unread["runs"][0]["id"], 2);
}
