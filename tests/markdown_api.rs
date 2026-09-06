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
) -> (StatusCode, serde_json::Value) {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if auth {
        r = r.header("x-auth-user", "test");
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
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (code, t) = request(
        app.clone(),
        "POST",
        "/api/tasks",
        json!({"title":"new"}),
        true,
    )
    .await;
    assert_eq!(code, StatusCode::CREATED);
    assert!(t["id"].is_string());
    assert_eq!(
        request(app.clone(), "POST", "/api/merges", json!({}), true)
            .await
            .0,
        StatusCode::GONE
    );
    let run =
        json!({"source":"worker","claim_id":"c","attempt":1,"task_id":t["id"],"note":"evidence"});
    let (_, a) = request(app.clone(), "POST", "/worker/runs", run.clone(), false).await;
    let (_, b) = request(app.clone(), "POST", "/worker/runs", run, false).await;
    assert_eq!(a["id"], b["id"]);
    let path = format!("/api/runs/{}/read", a["id"]);
    let (code, r) = request(app.clone(), "POST", &path, json!({"note":"filed"}), true).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(r["read_note"], "filed");
    let (_, snap) = request(app, "GET", "/worker/snapshot", json!({}), false).await;
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
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(original["body"], payload["report_markdown"]);
    assert_eq!(original["checks"][0]["exit_code"], 0);
    let (_, repeated) = request(app, "POST", "/worker/report", payload, false).await;
    assert_eq!(repeated["report_id"], reported["report_id"]);
    assert_eq!(state.store.list("runs").unwrap().len(), 1);
}

#[test]
fn legacy_product_documents_survive_restart_and_directory_changes() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let original = json!({"id":"org/repo","repository":"old","description":"old","releases":true,"archived":true,"archived_at":"2025-01-01","created_at":"created","updated_at":"updated","body":"migration body","legacy":{"sqlite_columns":{"extra":"preserve"}},"custom":{"nested":[1,2]}});
    {
        let store = Store::open(data.path()).unwrap();
        store.put("products", "org/repo", original.clone()).unwrap();
        store.put("tasks", "historic", json!({"id":"historic","product_id":"org/repo","archived":true,"status":"done","body":"history"})).unwrap();
    }
    let expected_bytes = std::fs::read(data.path().join("products/org%2Frepo.md")).unwrap();
    for present in [true, false, true] {
        let checkout = root.path().join("org/repo");
        if present {
            std::fs::create_dir_all(&checkout).unwrap();
        } else {
            std::fs::remove_dir_all(&checkout).unwrap();
        }
        let s = AppState::from_vars(|key| match key {
            "APP_DATA_DIR" => Some(data.path().to_string_lossy().into_owned()),
            "APP_PROJECTS_DIR" => Some(root.path().to_string_lossy().into_owned()),
            _ => None,
        })
        .unwrap();
        assert_eq!(s.store.get("products", "org/repo").unwrap(), original);
        assert_eq!(s.store.list("products").unwrap().len(), 1);
        assert_eq!(
            s.store.get("tasks", "historic").unwrap()["product_id"],
            "org/repo"
        );
        assert_eq!(
            std::fs::read(data.path().join("products/org%2Frepo.md")).unwrap(),
            expected_bytes
        );
    }
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
        let (status, list) = request(app.clone(), "GET", path, json!({}), true).await;
        assert_eq!(status, StatusCode::OK);
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].get("legacy").is_none());
        assert!(list[0].get("body").is_none());
        assert!(list[0].get("last_report").is_none());
    }
    let (_, detail) = request(app.clone(), "GET", "/api/tasks/draft", json!({}), true).await;
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
    let (_, first) = request(app.clone(), "GET", "/api/runs?limit=1", json!({}), true).await;
    assert_eq!(first["next"], 1);
    let (_, last) = request(
        app.clone(),
        "GET",
        "/api/runs?since=1&limit=1",
        json!({}),
        true,
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
    let (_, unread) = request(app, "GET", "/api/runs?unread=true", json!({}), true).await;
    assert_eq!(unread["runs"].as_array().unwrap().len(), 1);
    assert_eq!(unread["runs"][0]["id"], 2);
}

#[tokio::test]
async fn mcp_checkpoint_round_trip_new_session_and_expired_execution_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let clock = SharedClock::at(datetime!(2026-09-05 00:00 UTC));
    let state = AppState::new(Store::open(dir.path()).unwrap())
        .with_clock(Arc::new(clock.clone()))
        .with_ttl(10);
    state
        .store
        .put("products", "a/b", json!({"id":"a/b"}))
        .unwrap();
    task::create(&state, json!({"id":"t","title":"test","product_id":"a/b"})).unwrap();
    task::set_status(&state, "t", "ready").unwrap();
    let claim = task::claim(&state, "worker").unwrap().unwrap()["claim_id"].clone();
    let app = task_server::app(state.clone());
    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"checkpoint-test","version":"1"}}});
    let (_, headers, _) = rpc(app.clone(), None, init.clone()).await;
    let session = headers["mcp-session-id"].to_str().unwrap();
    let (_, _, tools) = rpc(
        app.clone(),
        Some(session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    let schema = &tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "task_checkpoint_update")
        .unwrap()["inputSchema"];
    assert!(schema["properties"]["claim_id"].is_object());
    assert!(schema["properties"]["expected_revision"].is_object());
    for (revision, set, delete) in [
        (
            0,
            json!({"ci_url":"https://example.test/runs/1","next_step":"wait_ci","temp":true}),
            json!([]),
        ),
        (1, json!({"next_step":"merge"}), json!(["temp"])),
    ] {
        let (_, _, result) = rpc(app.clone(), Some(session), json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"task_checkpoint_update","arguments":{"id":"t","claim_id":claim,"expected_revision":revision,"set":set,"delete_keys":delete}}})).await;
        assert_ne!(result["result"]["isError"], true, "{result}");
        assert_eq!(
            result["result"]["structuredContent"]["revision"],
            revision + 1
        );
    }
    clock.advance_secs(11);
    let (_, headers, _) = rpc(app.clone(), None, init).await;
    let session = headers["mcp-session-id"].to_str().unwrap();
    let (_, _, result) = rpc(app.clone(), Some(session), json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"task_checkpoint_get","arguments":{"id":"t","execution_id":claim}}})).await;
    let saved = &result["result"]["structuredContent"];
    assert!(saved["active_claim_id"].is_null());
    assert_eq!(
        saved["checkpoints"][0]["values"],
        json!({"ci_url":"https://example.test/runs/1","next_step":"merge"})
    );
    let (_, _, rejected) = rpc(app.clone(), Some(session), json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"task_checkpoint_update","arguments":{"id":"t","claim_id":claim,"expected_revision":2,"set":{"next_step":"wrong"}}}})).await;
    assert_eq!(rejected["result"]["isError"], true);
    let (_, _, missing) = rpc(app, Some(session), json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"task_checkpoint_get","arguments":{"id":"t","execution_id":"unknown"}}})).await;
    assert_eq!(missing["result"]["isError"], true);
}

#[tokio::test]
async fn explicit_product_mcp_updates_reach_http_and_worker_without_rescan() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(Store::open(dir.path()).unwrap());
    let app = task_server::app(state.clone());
    let (_, headers, _) = rpc(app.clone(), None, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"products-test","version":"1"}}})).await;
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
    let names = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(!names.contains(&"product_rescan"));
    for name in [
        "product_register",
        "product_get",
        "product_list",
        "product_update",
        "product_archive",
    ] {
        assert!(names.contains(&name));
    }
    for (name, args, expected) in [
        (
            "product_register",
            json!({"id":"stable/id","repository":"https://example/canonical/repo"}),
            json!({"releases":null}),
        ),
        (
            "product_update",
            json!({"id":"stable/id","releases":false,"local_path":"/different/checkout"}),
            json!({"releases":false,"local_path":"/different/checkout"}),
        ),
        (
            "product_update",
            json!({"id":"stable/id","description":"公開しない製品"}),
            json!({"releases":false,"local_path":"/different/checkout","description":"公開しない製品"}),
        ),
        (
            "product_archive",
            json!({"id":"stable/id"}),
            json!({"archived":true}),
        ),
        (
            "product_update",
            json!({"id":"stable/id","releases":null,"local_path":null}),
            json!({"releases":null,"local_path":null,"archived":true}),
        ),
    ] {
        let (_, _, result) = rpc(app.clone(), Some(session), json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":args}})).await;
        assert_ne!(result["result"]["isError"], true, "{result}");
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(
                &result["result"]["structuredContent"][key], value,
                "{name}: {key}"
            );
        }
    }
    let (_, _, got) = rpc(app.clone(), Some(session), json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"product_get","arguments":{"id":"stable/id"}}})).await;
    let expected = &got["result"]["structuredContent"];
    for (path, auth) in [
        ("/api/products/stable/id", true),
        ("/worker/products/stable%2Fid", false),
    ] {
        let (status, actual) = request(app.clone(), "GET", path, json!(null), auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&actual, expected);
    }
    let (_, list) = request(app.clone(), "GET", "/api/products", json!(null), true).await;
    assert_eq!(list[0]["archived"], true);
    assert_eq!(list[0]["repository"], "https://example/canonical/repo");
    let (status, _) = request(app, "POST", "/api/products/rescan", json!({}), true).await;
    assert_eq!(status, StatusCode::GONE);
}

async fn mcp_call(
    app: &axum::Router,
    session: &str,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    rpc(app.clone(), Some(session), json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":name,"arguments":args}})).await.2
}

async fn rejects_arguments(
    app: &axum::Router,
    session: &str,
    cases: Vec<(&str, serde_json::Value)>,
) {
    for (name, args) in cases {
        let result = mcp_call(app, session, name, args.clone()).await;
        assert!(
            result.get("error").is_some() || result["result"]["isError"] == true,
            "{name} {args}: {result}"
        );
    }
}

#[tokio::test]
async fn mcp_compact_reads_paginate_filter_and_reject_ignored_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(Store::open(dir.path()).unwrap());
    for (id, product, priority) in [("a", "a/b", 2), ("b", "a/b", 2), ("c", "c/d", 1)] {
        task::create(&state, json!({"id":id,"title":id,"body":"original task body","product_id":product,"priority":priority})).unwrap();
    }
    let prose = "legacy evidence".repeat(100);
    state
        .store
        .update("tasks", "a", |t| {
            t["verification"] = json!(prose);
            t["last_report"] = json!({"verification":prose,"claim_id":"old"});
            t["milestones"] = json!([{"name":"verified","evidence":prose}]);
            t["milestone_history"] = json!([{"name":"reviewed","evidence":"old evidence"}]);
            t["execution_checkpoints"] =
                json!([{"execution_id":"old","revision":1,"values":{"next_step":"resume here"}}]);
            Ok(())
        })
        .unwrap();
    let app = task_server::app(state.clone());
    let (_, headers, _) = rpc(app.clone(), None, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"compact-test","version":"1"}}})).await;
    let session = headers["mcp-session-id"].to_str().unwrap();
    let first = mcp_call(
        &app,
        session,
        "task_list",
        json!({"product_id":"a/b","status":"draft","limit":1}),
    )
    .await;
    let page = &first["result"]["structuredContent"];
    assert_eq!(page["total"], 2);
    assert_eq!(page["next_offset"], 1);
    assert_eq!(page["tasks"][0]["id"], "a");
    assert!(page["tasks"][0].get("verification").is_none());
    assert!(page["tasks"][0].get("body").is_none());
    let second = mcp_call(
        &app,
        session,
        "task_list",
        json!({"product_id":"a/b","status":"draft","limit":1,"offset":1}),
    )
    .await;
    assert_eq!(second["result"]["structuredContent"]["tasks"][0]["id"], "b");
    assert!(second["result"]["structuredContent"]["next_offset"].is_null());
    let got = mcp_call(&app, session, "task_get", json!({"id":"a"})).await;
    let card = &got["result"]["structuredContent"];
    assert_eq!(card["body"], "original task body");
    for key in [
        "last_report",
        "verification",
        "milestones",
        "milestone_history",
        "execution_checkpoints",
    ] {
        assert!(card.get(key).is_none(), "{key}");
    }
    let history = mcp_call(&app, session, "task_history", json!({"id":"a","limit":1})).await;
    assert!(
        history["result"]["structuredContent"]["total"]
            .as_u64()
            .unwrap()
            >= 3
    );
    let checkpoint = mcp_call(
        &app,
        session,
        "task_checkpoint_get",
        json!({"id":"a","execution_id":"old"}),
    )
    .await;
    assert_eq!(
        checkpoint["result"]["structuredContent"]["checkpoints"][0]["values"]["next_step"],
        "resume here"
    );
    let updated = mcp_call(
        &app,
        session,
        "task_update",
        json!({"id":"a","body":"replacement"}),
    )
    .await;
    assert_eq!(updated["result"]["structuredContent"]["ok"], true);
    assert!(updated["result"]["structuredContent"].get("body").is_none());
    assert_eq!(task::card(&state, "a").unwrap()["verification"], prose);
    assert_eq!(task::card(&state, "a").unwrap()["body"], "replacement");
}

#[tokio::test]
async fn mcp_product_run_and_checkpoint_pages_preserve_originals() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(Store::open(dir.path()).unwrap());
    for (id, archived) in [("a/one", false), ("a/two", false), ("z/old", true)] {
        state.store.put("products",id,json!({"id":id,"repository":"https://example.test/repo","description":id,"archived":archived})).unwrap();
    }
    task::create(
        &state,
        json!({"id":"t","title":"task","product_id":"a/one"}),
    )
    .unwrap();
    state
        .store
        .update("tasks", "t", |t| {
            t["execution_checkpoints"] = json!([
                {"execution_id":"one","revision":1,"values":{"next_step":"first"}},
                {"execution_id":"two","revision":2,"values":{"next_step":"second"}}
            ]);
            Ok(())
        })
        .unwrap();
    for (id, task, source, read) in [
        (1, "t", "worker", false),
        (2, "other", "worker", false),
        (3, "t", "rescue", false),
        (4, "t", "worker", true),
        (5, "t", "worker", false),
    ] {
        state.store.put("runs",&id.to_string(),json!({"id":id,"task_id":task,"product_id":"a/one","source":source,"read_at":if read { json!("2026-09-05") } else { json!(null) },"body":format!("original {id}"),"note":format!("note {id}")})).unwrap();
    }
    let app = task_server::app(state);
    let (_, headers, _) = rpc(app.clone(),None,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"pages-test","version":"1"}}})).await;
    let session = headers["mcp-session-id"].to_str().unwrap();
    let p = mcp_call(
        &app,
        session,
        "product_list",
        json!({"archived":false,"limit":1,"offset":1}),
    )
    .await;
    assert_eq!(
        p["result"]["structuredContent"]["products"][0]["id"],
        "a/two"
    );
    assert_eq!(p["result"]["structuredContent"]["total"], 2);
    assert!(p["result"]["structuredContent"]["next_offset"].is_null());
    let p = mcp_call(&app, session, "product_list", json!({"archived":true})).await;
    assert_eq!(
        p["result"]["structuredContent"]["products"][0]["id"],
        "z/old"
    );
    for (offset, id, next) in [(0, 1, json!(1)), (1, 5, json!(null))] {
        let r = mcp_call(&app,session,"run_list",json!({"task_id":"t","product_id":"a/one","source":"worker","unread":true,"limit":1,"offset":offset})).await;
        let page = &r["result"]["structuredContent"];
        assert_eq!(page["total"], 2);
        assert_eq!(page["next_offset"], next);
        assert_eq!(page["runs"][0]["id"], id);
        assert!(page["runs"][0].get("body").is_none());
        let r = mcp_call(&app, session, "run_get", json!({"id":id.to_string()})).await;
        assert_eq!(
            r["result"]["structuredContent"]["body"],
            format!("original {id}")
        );
        assert_eq!(
            r["result"]["structuredContent"]["note"],
            format!("note {id}")
        );
    }
    let r = mcp_call(&app, session, "run_list", json!({"unread":false})).await;
    assert_eq!(r["result"]["structuredContent"]["runs"][0]["id"], 4);
    let p = mcp_call(
        &app,
        session,
        "task_checkpoint_get",
        json!({"id":"t","limit":1,"offset":1}),
    )
    .await;
    assert_eq!(
        p["result"]["structuredContent"]["checkpoints"][0]["values"]["next_step"],
        "second"
    );
    assert_eq!(p["result"]["structuredContent"]["total"], 2);
    for (name, args) in [
        ("task_checkpoint_get", json!({"id":"t","limit":0})),
        ("run_list", json!({"unread":"true"})),
        ("run_list", json!({"status":"done"})),
        ("task_list", json!({"limit":null})),
        ("task_update", json!({"id":"t","title":null})),
        ("task_update", json!({"id":"t","product_id":42})),
        ("product_list", json!({"archived":"false"})),
    ] {
        let r = mcp_call(&app, session, name, args.clone()).await;
        assert!(
            r.get("error").is_some() || r["result"]["isError"] == true,
            "{name} {args}: {r}"
        );
    }
    let empty = mcp_call(&app, session, "run_list", json!({"offset":999})).await;
    assert_eq!(empty["result"]["structuredContent"]["runs"], json!([]));
    assert!(empty["result"]["structuredContent"]["next_offset"].is_null());
}

#[tokio::test]
async fn mcp_execution_filter_finds_checkpoints_beyond_default_page() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(Store::open(dir.path()).unwrap());
    task::create(&state, json!({"id":"t","title":"long execution history"})).unwrap();
    state.store.update("tasks","t",|t| {
        t["execution_checkpoints"] = json!((0..55).map(|i| json!({"execution_id":format!("execution-{i}"),"revision":i,"values":{"next_step":format!("step-{i}")}})).collect::<Vec<_>>());
        Ok(())
    }).unwrap();
    let app = task_server::app(state);
    let (_, headers, _) = rpc(app.clone(),None,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"execution-test","version":"1"}}})).await;
    let session = headers["mcp-session-id"].to_str().unwrap();
    let first = mcp_call(&app, session, "task_checkpoint_get", json!({"id":"t"})).await;
    assert_eq!(
        first["result"]["structuredContent"]["checkpoints"]
            .as_array()
            .unwrap()
            .len(),
        50
    );
    assert_eq!(first["result"]["structuredContent"]["next_offset"], 50);
    let last = mcp_call(
        &app,
        session,
        "task_checkpoint_get",
        json!({"id":"t","execution_id":"execution-54"}),
    )
    .await;
    assert_eq!(last["result"]["structuredContent"]["total"], 1);
    assert_eq!(
        last["result"]["structuredContent"]["checkpoints"][0]["values"]["next_step"],
        "step-54"
    );
}

#[tokio::test]
async fn mcp_tool_schemas_reject_ignored_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let app = task_server::app(AppState::new(Store::open(dir.path()).unwrap()));
    let (_, headers, _) = rpc(app.clone(),None,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"validation-test","version":"1"}}})).await;
    let session = headers["mcp-session-id"].to_str().unwrap();
    rejects_arguments(
        &app,
        session,
        vec![
            ("task_list", json!({"title":"ignored"})),
            ("task_list", json!({"status":"bogus"})),
            ("task_list", json!({"product_id":"invalid"})),
            ("task_list", json!({"limit":0})),
            ("task_list", json!({"limit":201})),
            ("task_list", json!({"offset":-1})),
            ("task_get", json!({"id":"a","status":"draft"})),
            ("task_update", json!({"id":"a","status":"done"})),
            ("task_create", json!({"title":"x","commit_sha":"ignored"})),
            ("product_list", json!({"unknown":true})),
            (
                "task_checkpoint_update",
                json!({"id":"a","claim_id":"old","expected_revision":0,"unknown":true}),
            ),
        ],
    )
    .await;
}

#[test]
fn production_starts_without_browser_token_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::from_vars(|key| match key {
        "TASK_SERVER_ENV" => Some("production".into()),
        "APP_DATA_DIR" => Some(dir.path().to_string_lossy().into_owned()),
        _ => None,
    })
    .unwrap();
    assert_eq!(state.claim_ttl_secs, 3600);
    assert!(state.dev_identity.is_none());
}

#[tokio::test]
async fn browser_writes_preserve_validation_and_state_constraints() {
    let dir = tempfile::tempdir().unwrap();
    let app = task_server::app(AppState::new(Store::open(dir.path()).unwrap()));
    let (code, session) = request(app.clone(), "GET", "/api/session", json!({}), true).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(session, json!({"user":"test"}));
    let (code, _) = request(app.clone(), "POST", "/api/tasks", json!({"title":""}), true).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let (code, _) = request(
        app.clone(),
        "POST",
        "/api/tasks",
        json!({"id":"browser-write","title":"new"}),
        true,
    )
    .await;
    assert_eq!(code, StatusCode::CREATED);
    let (code, card) = request(
        app.clone(),
        "PATCH",
        "/api/tasks/browser-write",
        json!({"title":"edited"}),
        true,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(card["title"], "edited");
    let (code, _) = request(
        app.clone(),
        "POST",
        "/api/tasks/browser-write/status",
        json!({"status":"wip"}),
        true,
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT);
    let (code, _) = request(
        app.clone(),
        "DELETE",
        "/api/tasks/browser-write",
        json!({}),
        true,
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT);
    let (code, card) = request(
        app.clone(),
        "POST",
        "/api/tasks/browser-write/status",
        json!({"status":"cancelled"}),
        true,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(card["status"], "cancelled");
    let (code, _) = request(app, "DELETE", "/api/tasks/browser-write", json!({}), true).await;
    assert_eq!(code, StatusCode::NO_CONTENT);
}
