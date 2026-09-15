use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::path::Path;
use task_server::{AppState, task};
use tower::ServiceExt;

fn state(root: &Path, config: Option<&str>) -> Result<AppState, task_server::Error> {
    AppState::from_vars(|key| match key {
        "APP_DATA_DIR" => Some(root.join("ledger").to_string_lossy().into_owned()),
        "EXECUTION_TARGETS_FILE" => config.map(str::to_owned),
        _ => None,
    })
}

async fn request(s: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = task_server::app(s.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

#[tokio::test]
async fn unconfigured_targets_leave_existing_records_readable_and_editable() {
    let root = tempfile::tempdir().unwrap();
    let s = state(root.path(), None).unwrap();
    assert_eq!(
        request(&s, "GET", "/api/execution-targets", json!(null)).await,
        (StatusCode::OK, json!({"labels":[],"default":null}))
    );
    for body in [
        json!({"title":"new"}),
        json!({"title":"new","execution_target":"forge"}),
    ] {
        assert_eq!(
            request(&s, "POST", "/api/tasks", body).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    for body in [
        json!({"worker":"old"}),
        json!({"worker":"new","execution_target":"forge"}),
    ] {
        assert_eq!(
            request(&s, "POST", "/worker/claim", body).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    for (id, target) in [("legacy", None), ("removed", Some("old queue"))] {
        let mut original = json!({"id":id,"title":"original","status":"done","product_id":"a/b","body":"keep","custom":17});
        if let Some(target) = target {
            original["execution_target"] = json!(target);
        }
        s.store.put("tasks", id, original.clone()).unwrap();
        let (_, card) = request(&s, "GET", &format!("/api/tasks/{id}"), json!(null)).await;
        assert_eq!(card["execution_target"], json!(target));
        assert_eq!(s.store.get("tasks", id).unwrap(), original);
        assert_eq!(
            request(
                &s,
                "PATCH",
                &format!("/api/tasks/{id}"),
                json!({"title":"edited"})
            )
            .await
            .0,
            StatusCode::OK
        );
        let saved = s.store.get("tasks", id).unwrap();
        assert_eq!(
            saved.get("execution_target"),
            original.get("execution_target")
        );
        assert_eq!(saved["custom"], 17);
    }
    for path in ["/api/tasks?status=done", "/api/done", "/api/closed"] {
        let (code, rows) = request(&s, "GET", path, json!(null)).await;
        assert_eq!(code, StatusCode::OK);
        let rows = rows.as_array().unwrap();
        assert_eq!(
            rows.iter().find(|t| t["id"] == "legacy").unwrap()["execution_target"],
            Value::Null
        );
        assert_eq!(
            rows.iter().find(|t| t["id"] == "removed").unwrap()["execution_target"],
            "old queue"
        );
    }
    let (code, filtered) = request(
        &s,
        "GET",
        "/api/tasks?status=done&execution_target=old%20queue",
        json!(null),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(filtered.as_array().unwrap().len(), 1);
    assert_eq!(filtered[0]["id"], "removed");
}

#[test]
fn explicit_configuration_errors_fail_startup() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("targets.yaml");
    assert!(state(root.path(), Some("")).is_err());
    assert!(state(root.path(), Some(path.to_str().unwrap())).is_err());
    for text in [
        "",
        "labels: [",
        "{}",
        "labels: null",
        "labels: queue",
        "labels: [7]",
        "labels: ['']",
        "labels: ['  ']",
        "labels: [forge, forge]",
        "labels: [forge]\ndefault: field",
        "labels: [forge]\ndefault: 7",
        "labels: [forge]\nunknown: true",
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(
            state(root.path(), Some(path.to_str().unwrap())).is_err(),
            "accepted {text:?}"
        );
    }
}

fn configured() -> (tempfile::TempDir, std::path::PathBuf, AppState) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("targets.yaml");
    std::fs::write(
        &path,
        "labels: [forge, field, '研究 / 試行']\ndefault: forge\n",
    )
    .unwrap();
    let state = state(root.path(), Some(path.to_str().unwrap())).unwrap();
    (root, path, state)
}

#[tokio::test]
async fn configured_targets_route_http_requests() {
    let (_root, _path, s) = configured();
    s.store.put("products", "a/b", json!({"id":"a/b"})).unwrap();
    for (id, label, filter) in [
        ("one", "forge", "forge"),
        ("two", "field", "field"),
        (
            "three",
            "研究 / 試行",
            "%E7%A0%94%E7%A9%B6%20%2F%20%E8%A9%A6%E8%A1%8C",
        ),
    ] {
        let (code, created) = request(
            &s,
            "POST",
            "/api/tasks",
            json!({"id":id,"title":"new","product_id":"a/b","execution_target":label}),
        )
        .await;
        assert_eq!(code, StatusCode::CREATED);
        assert_eq!(created["execution_target"], label);
        let (code, rows) = request(
            &s,
            "GET",
            &format!("/api/tasks?execution_target={filter}"),
            json!(null),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["id"], id);
        task::set_status(&s, id, "ready").unwrap();
        let (code, _) = request(&s, "POST", "/worker/claim", json!({"worker":"wrong","task_id":id,"execution_target":if label == "field" {"forge"} else {"field"}})).await;
        assert_eq!(code, StatusCode::NO_CONTENT);
        let mut body = json!({"worker":"own","execution_target":label});
        if id != "two" {
            body["task_id"] = json!(id);
        }
        let (code, claim) = request(&s, "POST", "/worker/claim", body).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(claim["task"]["id"], id);
        assert_eq!(claim["task"]["execution_target"], label);
        assert_eq!(
            request(
                &s,
                "POST",
                "/worker/claim",
                json!({"worker":"again","execution_target":label})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            request(
                &s,
                "POST",
                "/worker/heartbeat",
                json!({"claim_id":claim["claim_id"]})
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            request(
                &s,
                "POST",
                "/worker/report",
                json!({"claim_id":claim["claim_id"],"outcome":"done","report_markdown":"verified"})
            )
            .await
            .0,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn legacy_default_and_definitions_stay_external() {
    let (root, path, s) = configured();
    let config = std::fs::read_to_string(&path).unwrap();
    s.store.put("products", "a/b", json!({"id":"a/b"})).unwrap();
    let (_, created) = request(
        &s,
        "POST",
        "/api/tasks",
        json!({"id":"default","title":"default"}),
    )
    .await;
    assert_eq!(created["execution_target"], "forge");
    s.store
        .put(
            "tasks",
            "legacy",
            json!({"id":"legacy","title":"old","status":"ready","product_id":"a/b"}),
        )
        .unwrap();
    let legacy_bytes = std::fs::read(root.path().join("ledger/tasks/legacy.md")).unwrap();
    for endpoint in ["/api/tasks/legacy", "/api/tasks?execution_target=forge"] {
        assert_eq!(
            request(&s, "GET", endpoint, json!(null)).await.0,
            StatusCode::OK
        );
    }
    assert_eq!(
        std::fs::read(root.path().join("ledger/tasks/legacy.md")).unwrap(),
        legacy_bytes
    );
    let (code, claim) = request(
        &s,
        "POST",
        "/worker/claim",
        json!({"worker":"legacy","task_id":"legacy"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(claim["task"]["execution_target"], "forge");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), config);
    let snapshot = request(&s, "GET", "/worker/snapshot", json!(null)).await.1;
    assert_eq!(
        snapshot
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["archive", "claim_receipts", "products", "runs", "tasks"]
    );
}

#[tokio::test]
async fn config_changes_only_take_effect_after_restart() {
    let (root, path, s) = configured();
    s.store
        .put(
            "tasks",
            "one",
            json!({"id":"one","title":"old","execution_target":"forge","status":"done"}),
        )
        .unwrap();
    task::create(&s, json!({"id":"default","title":"default"})).unwrap();
    let changed_config = "labels: [field, island]\n";
    std::fs::write(&path, changed_config).unwrap();
    assert_eq!(
        request(&s, "GET", "/api/execution-targets", json!(null))
            .await
            .1["default"],
        "forge"
    );
    drop(s);
    let s = state(root.path(), Some(path.to_str().unwrap())).unwrap();
    assert_eq!(
        request(&s, "GET", "/api/execution-targets", json!(null))
            .await
            .1,
        json!({"labels":["field","island"],"default":null})
    );
    assert_eq!(
        request(&s, "POST", "/api/tasks", json!({"title":"missing"}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&s, "POST", "/worker/claim", json!({"worker":"missing"}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &s,
            "POST",
            "/api/tasks",
            json!({"title":"new","execution_target":"island"})
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let (code, rows) = request(
        &s,
        "GET",
        "/api/tasks?status=done&execution_target=forge",
        json!(null),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(rows[0]["id"], "one");
    assert_eq!(
        request(&s, "PATCH", "/api/tasks/one", json!({"title":"retained"}))
            .await
            .1["execution_target"],
        "forge"
    );
    assert_eq!(
        request(
            &s,
            "PATCH",
            "/api/tasks/one",
            json!({"execution_target":"forge"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &s,
            "PATCH",
            "/api/tasks/one",
            json!({"execution_target":"island"})
        )
        .await
        .1["execution_target"],
        "island"
    );
    assert_eq!(
        request(
            &s,
            "POST",
            "/worker/claim",
            json!({"worker":"removed","execution_target":"forge","task_id":"default"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed_config);
}
