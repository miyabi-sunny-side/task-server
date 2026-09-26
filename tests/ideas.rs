mod common;
use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;
use task_server::{AppState, Error, SharedClock, idea, ledger::Store, task};
use time::macros::datetime;
use tower::ServiceExt;

fn clocked(dir: &std::path::Path) -> (AppState, SharedClock) {
    let clock = SharedClock::at(datetime!(2026-09-26 00:00 UTC));
    let state = common::state(Store::open(dir).unwrap()).with_clock(Arc::new(clock.clone()));
    (state, clock)
}

fn ids(values: &[Value]) -> Vec<&str> {
    values.iter().map(|v| v["id"].as_str().unwrap()).collect()
}

#[test]
fn title_only_ideas_persist_and_list_by_latest_update() {
    let dir = tempfile::tempdir().unwrap();
    let (s, clock) = clocked(dir.path());
    let first = idea::create(&s, json!({"title":"Only a title"})).unwrap();
    assert_eq!(first["body"], "");
    assert!(first["product_id"].is_null());
    assert_eq!(first["revision"], 1);
    assert!(
        dir.path()
            .join("idea")
            .join(format!("{}.md", first["id"].as_str().unwrap()))
            .is_file()
    );
    clock.advance_secs(1);
    let b = idea::create(
        &s,
        json!({"title":"b","product_id":"org/repo","body":"notes"}),
    )
    .unwrap();
    let c = idea::create(&s, json!({"title":"c"})).unwrap();
    clock.advance_secs(1);
    let mut hand = s.store.get("idea", first["id"].as_str().unwrap()).unwrap();
    hand["custom"] = json!({"keep":true});
    s.store
        .put("idea", first["id"].as_str().unwrap(), hand)
        .unwrap();
    idea::update(
        &s,
        first["id"].as_str().unwrap(),
        json!({"expected_revision":1,"body":"researched"}),
    )
    .unwrap();

    let (tied_first, tied_second) = if b["id"].as_str() < c["id"].as_str() {
        (&b, &c)
    } else {
        (&c, &b)
    };
    let listed = idea::list(&s, false).unwrap();
    assert_eq!(
        ids(&listed),
        [
            first["id"].as_str().unwrap(),
            tied_first["id"].as_str().unwrap(),
            tied_second["id"].as_str().unwrap()
        ]
    );
    assert!(listed.iter().all(|i| i.get("body").is_none()));

    drop(s);
    let reopened = common::state(Store::open(dir.path()).unwrap());
    let before = reopened
        .store
        .get("idea", first["id"].as_str().unwrap())
        .unwrap();
    let got = idea::get(&reopened, first["id"].as_str().unwrap()).unwrap();
    assert_eq!(got["body"], "researched");
    assert_eq!(got["custom"], json!({"keep":true}));
    assert_eq!(got["updated_at"], "2026-09-26T00:00:02Z");
    assert_eq!(
        reopened
            .store
            .get("idea", first["id"].as_str().unwrap())
            .unwrap(),
        before
    );

    for invalid in [
        json!({"title":"  "}),
        json!({"title":"x","execution_target":"forge"}),
        json!({"title":"x","product_id":"not a product"}),
        json!({"title":"x","id":"chosen"}),
    ] {
        assert!(matches!(
            idea::create(&reopened, invalid),
            Err(Error::Invalid(_))
        ));
    }
    assert_eq!(idea::list(&reopened, false).unwrap().len(), 3);
}

#[test]
fn stale_revision_cannot_overwrite_another_writers_append() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = clocked(dir.path());
    let id = idea::create(&s, json!({"title":"shared","body":"base"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reader_a = idea::get(&s, &id).unwrap();
    let reader_b = idea::get(&s, &id).unwrap();
    let saved = idea::update(
        &s,
        &id,
        json!({"expected_revision":reader_a["revision"],"body":"base\nA research"}),
    )
    .unwrap();
    assert_eq!(saved["revision"], 2);
    let stale = idea::update(
        &s,
        &id,
        json!({"expected_revision":reader_b["revision"],"body":"base\nB research"}),
    );
    assert!(matches!(stale, Err(Error::Conflict(_))));
    assert_eq!(idea::get(&s, &id).unwrap()["body"], "base\nA research");
    let merged = idea::update(
        &s,
        &id,
        json!({"expected_revision":2,"body":"base\nA research\nB research","product_id":"org/repo"}),
    )
    .unwrap();
    assert_eq!(merged["revision"], 3);
    let cleared = idea::update(&s, &id, json!({"expected_revision":3,"product_id":null})).unwrap();
    assert!(cleared["product_id"].is_null());
    for invalid in [
        json!({"body":"no revision"}),
        json!({"expected_revision":4,"title":""}),
        json!({"expected_revision":4,"revision":9}),
        json!({"expected_revision":4,"archived":false}),
    ] {
        assert!(matches!(
            idea::update(&s, &id, invalid),
            Err(Error::Invalid(_))
        ));
    }
    assert!(matches!(
        idea::update(&s, "absent", json!({"expected_revision":1})),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn archive_moves_idea_to_the_archive_list_and_makes_it_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let (s, clock) = clocked(dir.path());
    let id = idea::create(&s, json!({"title":"maybe later"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    clock.advance_secs(5);
    let archived = idea::archive(&s, &id).unwrap();
    assert_eq!(archived["archived"], true);
    assert_eq!(archived["archived_at"], "2026-09-26T00:00:05Z");
    clock.advance_secs(5);
    assert_eq!(idea::archive(&s, &id).unwrap(), archived);
    assert!(idea::list(&s, false).unwrap().is_empty());
    assert_eq!(ids(&idea::list(&s, true).unwrap()), [id.as_str()]);
    assert_eq!(idea::get(&s, &id).unwrap()["title"], "maybe later");
    assert!(matches!(
        idea::update(
            &s,
            &id,
            json!({"expected_revision":archived["revision"],"body":"x"})
        ),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        idea::promote(&s, &id, json!({"execution_target":"forge"})),
        Err(Error::Conflict(_))
    ));
}

#[test]
fn promotion_creates_one_linked_draft_and_survives_an_interrupted_link() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = clocked(dir.path());
    let created = idea::create(
        &s,
        json!({"title":"grow me","body":"research notes","product_id":"org/repo"}),
    )
    .unwrap();
    let id = created["id"].as_str().unwrap().to_owned();
    let refused = idea::promote(&s, &id, json!({"execution_target":"not configured"}));
    assert!(matches!(refused, Err(Error::Invalid(_))));
    assert!(s.store.list("tasks").unwrap().is_empty());
    assert_eq!(idea::get(&s, &id).unwrap(), created);

    let promoted = idea::promote(
        &s,
        &id,
        json!({"execution_target":"field","body":"scope and done conditions"}),
    )
    .unwrap();
    let task_id = promoted["task"]["id"].as_str().unwrap().to_owned();
    assert_eq!(promoted["task"]["status"], "draft");
    assert_eq!(promoted["task"]["execution_target"], "field");
    assert_eq!(promoted["task"]["title"], "grow me");
    assert_eq!(promoted["task"]["product_id"], "org/repo");
    assert_eq!(promoted["task"]["body"], "scope and done conditions");
    assert_eq!(promoted["task"]["idea_id"], id.as_str());
    assert_eq!(promoted["idea"]["task_id"], task_id.as_str());
    assert_eq!(promoted["idea"]["body"], "research notes");
    assert_eq!(promoted["idea"]["revision"], 2);

    let again =
        idea::promote(&s, &id, json!({"execution_target":"forge","title":"other"})).unwrap();
    assert_eq!(again["task"]["id"], task_id.as_str());
    assert_eq!(again["task"]["execution_target"], "field");
    assert_eq!(again["idea"]["revision"], 2);

    // The task write landed but the idea link did not (process stopped between files).
    let mut unlinked = idea::get(&s, &id).unwrap();
    unlinked["task_id"] = Value::Null;
    unlinked["promoted_at"] = Value::Null;
    s.store.put("idea", &id, unlinked).unwrap();
    let resumed = idea::promote(&s, &id, json!({"execution_target":"forge"})).unwrap();
    assert_eq!(resumed["task"]["id"], task_id.as_str());
    assert_eq!(resumed["idea"]["task_id"], task_id.as_str());
    assert_eq!(s.store.list("tasks").unwrap().len(), 1);

    let defaults = idea::create(&s, json!({"title":"default body","body":"kept"})).unwrap();
    let promoted = idea::promote(&s, defaults["id"].as_str().unwrap(), json!({})).unwrap();
    assert_eq!(promoted["task"]["execution_target"], "forge");
    assert_eq!(promoted["task"]["body"], "kept");
    assert!(promoted["task"]["product_id"].is_null());

    s.store
        .put(
            "products",
            "org/repo",
            json!({"id":"org/repo","repository":"x"}),
        )
        .unwrap();
    assert!(task::claim(&s, "worker", None, "field").unwrap().is_none());
    task::set_status(&s, &task_id, "ready").unwrap();
    let claim = task::claim(&s, "worker", None, "field").unwrap().unwrap();
    assert_eq!(claim["task"]["id"], task_id.as_str());
    assert!(task::claim(&s, "worker", None, "forge").unwrap().is_none());
}

async fn request(app: axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let r = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    let r = app
        .oneshot(r.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

async fn rpc(app: axum::Router, session: Option<&str>, message: Value) -> (HeaderMap, Value) {
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
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let value = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("data: ")
                .and_then(|j| serde_json::from_str(j).ok())
        })
        .or_else(|| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    (headers, value)
}

async fn tool(app: &axum::Router, session: &str, name: &str, arguments: Value) -> (bool, Value) {
    let (_, reply) = rpc(
        app.clone(),
        Some(session),
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":name,"arguments":arguments}}),
    )
    .await;
    let result = &reply["result"];
    (
        result["isError"] == true,
        result["structuredContent"].clone(),
    )
}

async fn mcp_session(app: &axum::Router) -> String {
    let (headers, _) = rpc(
        app.clone(),
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"idea-test","version":"1"}}}),
    )
    .await;
    let session = headers
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    rpc(
        app.clone(),
        Some(&session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    session
}

#[tokio::test]
async fn http_and_mcp_edit_the_same_idea_and_share_conflict_detection() {
    let dir = tempfile::tempdir().unwrap();
    let s = common::state(Store::open(dir.path()).unwrap());
    let app = task_server::app(s.clone());
    let (code, created) = request(
        app.clone(),
        "POST",
        "/api/ideas",
        json!({"title":"from UI"}),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_owned();

    let session = mcp_session(&app).await;

    let (error, listed) = tool(&app, &session, "idea_list", json!({"limit":1})).await;
    assert!(!error);
    assert_eq!(listed["ideas"][0]["id"], id.as_str());
    assert!(listed["ideas"][0].get("body").is_none());
    assert_eq!(listed["total"], 1);
    let (_, got) = tool(&app, &session, "idea_get", json!({"id":id})).await;
    assert_eq!(got["revision"], 1);
    let (error, updated) = tool(
        &app,
        &session,
        "idea_update",
        json!({"id":id,"expected_revision":1,"body":"agent research https://example.invalid"}),
    )
    .await;
    assert!(!error);
    assert_eq!(updated["revision"], 2);

    let (code, stale) = request(
        app.clone(),
        "PATCH",
        &format!("/api/ideas/{id}"),
        json!({"expected_revision":1,"body":"browser draft"}),
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(stale["code"], "conflict");
    let (code, fresh) = request(app.clone(), "GET", &format!("/api/ideas/{id}"), json!(null)).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(fresh["body"], "agent research https://example.invalid");
    let (code, _) = request(
        app.clone(),
        "PATCH",
        &format!("/api/ideas/{id}"),
        json!({"expected_revision":2,"body":"merged in browser"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (error, stale) = tool(
        &app,
        &session,
        "idea_update",
        json!({"id":id,"expected_revision":2,"body":"agent overwrite"}),
    )
    .await;
    assert!(error);
    assert_eq!(stale["code"], "conflict");
    let (error, _) = tool(
        &app,
        &session,
        "idea_update",
        json!({"id":id,"expected_revision":3,"unknown":1}),
    )
    .await;
    assert!(error);
}

#[tokio::test]
async fn http_and_mcp_promote_archive_and_export_the_same_ideas() {
    let dir = tempfile::tempdir().unwrap();
    let s = common::state(Store::open(dir.path()).unwrap());
    let app = task_server::app(s.clone());
    let (_, created) = request(
        app.clone(),
        "POST",
        "/api/ideas",
        json!({"title":"from UI"}),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_owned();
    let session = mcp_session(&app).await;

    let (error, created) = tool(
        &app,
        &session,
        "idea_create",
        json!({"title":"from agent","product_id":"org/repo"}),
    )
    .await;
    assert!(!error);
    let agent_idea = created["id"].as_str().unwrap().to_owned();
    let (code, promoted) = request(
        app.clone(),
        "POST",
        &format!("/api/ideas/{agent_idea}/promote"),
        json!({"execution_target":"field"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(promoted["task"]["idea_id"], agent_idea.as_str());
    let (_, card) = request(
        app.clone(),
        "GET",
        &format!("/api/tasks/{}", promoted["task"]["id"].as_str().unwrap()),
        json!(null),
    )
    .await;
    assert_eq!(card["idea_id"], agent_idea.as_str());
    let (error, again) = tool(
        &app,
        &session,
        "idea_promote",
        json!({"id":agent_idea,"execution_target":"forge"}),
    )
    .await;
    assert!(!error);
    assert_eq!(again["task_id"], promoted["task"]["id"]);

    let (error, archived) = tool(&app, &session, "idea_archive", json!({"id":id})).await;
    assert!(!error);
    assert_eq!(archived["archived"], true);
    let (_, active) = request(app.clone(), "GET", "/api/ideas", json!(null)).await;
    assert_eq!(ids(active.as_array().unwrap()), [agent_idea.as_str()]);
    let (_, old) = request(app.clone(), "GET", "/api/ideas?archived=true", json!(null)).await;
    assert_eq!(ids(old.as_array().unwrap()), [id.as_str()]);
    let (_, archived_list) = tool(&app, &session, "idea_list", json!({"archived":true})).await;
    assert_eq!(archived_list["ideas"][0]["id"], id.as_str());
    let (code, _) = request(app.clone(), "GET", "/api/ideas/absent", json!(null)).await;
    assert_eq!(code, StatusCode::NOT_FOUND);

    let (_, snapshot) = request(app, "GET", "/worker/snapshot", json!(null)).await;
    assert_eq!(snapshot["idea"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["tasks"].as_array().unwrap().len(), 1);
}
