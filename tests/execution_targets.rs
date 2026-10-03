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
        "labels: [Forge]",
        "labels: ['a/b']",
        "labels: ['a.b']",
        "labels: ['-x']",
        "labels: ['研究']",
        "labels: [\"a'b\"]",
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
    std::fs::write(&path, "labels: [forge, field, lab_2]\ndefault: forge\n").unwrap();
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
        ("three", "lab_2", "lab_2"),
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

#[test]
fn labels_allow_only_lowercase_digits_hyphen_and_underscore() {
    use task_server::execution_target::check_label;
    for label in ["sandbox", "game", "a-b_1", "0x", "9"] {
        assert!(check_label(label).is_ok(), "rejected {label:?}");
    }
    for label in [
        "", "-x", "_x", "Forge", "a b", "a/b", "a.b", "..", "a\"b", "a'b", "研究", "x\n", "a;b",
    ] {
        assert!(check_label(label).is_err(), "accepted {label:?}");
    }
}

fn targets_of(root: &Path) -> String {
    std::fs::read_to_string(root.join("ledger/settings/execution_targets.md")).unwrap()
}

#[tokio::test]
async fn definitions_live_in_the_ledger_and_snapshot() {
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
        snapshot["settings"],
        json!([{"id":"execution_targets","body":"","labels":["forge","field","lab_2"],"default":"forge"}])
    );
}

#[tokio::test]
async fn the_external_file_is_imported_only_once() {
    let (root, path, s) = configured();
    let imported = json!({"labels":["forge","field","lab_2"],"default":"forge"});
    assert_eq!(
        request(&s, "GET", "/api/execution-targets", json!(null))
            .await
            .1,
        imported
    );
    drop(s);
    for changed in ["labels: [field, island]\n", "labels: [Invalid / name]\n"] {
        std::fs::write(&path, changed).unwrap();
        let s = state(root.path(), Some(path.to_str().unwrap())).unwrap();
        assert_eq!(
            request(&s, "GET", "/api/execution-targets", json!(null))
                .await
                .1,
            imported
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
    }
    let before = targets_of(root.path());
    let s = state(root.path(), None).unwrap();
    assert_eq!(
        request(&s, "GET", "/api/execution-targets", json!(null))
            .await
            .1,
        imported
    );
    assert_eq!(targets_of(root.path()), before);
}

async fn code(s: &AppState, method: &str, path: &str, body: Value) -> StatusCode {
    request(s, method, path, body).await.0
}

#[tokio::test]
async fn labels_are_created_and_deleted_without_restart() {
    let (root, path, s) = configured();
    s.store.put("products", "a/b", json!({"id":"a/b"})).unwrap();
    let targets = "/api/execution-targets";
    let (code_, created) = request(&s, "POST", targets, json!({"label":"game"})).await;
    assert_eq!(code_, StatusCode::CREATED);
    assert_eq!(
        created,
        json!({"labels":["forge","field","lab_2","game"],"default":"forge"})
    );
    for (body, expected) in [
        (json!({"label":"game"}), StatusCode::CONFLICT),
        (json!({"label":"Game"}), StatusCode::BAD_REQUEST),
        (json!({"label":"../x"}), StatusCode::BAD_REQUEST),
        (json!({"label":7}), StatusCode::BAD_REQUEST),
        (json!({}), StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(
            code(&s, "POST", targets, body.clone()).await,
            expected,
            "{body}"
        );
    }
    assert_eq!(request(&s, "GET", targets, json!(null)).await.1, created);
    let task = json!({"id":"played","title":"play","product_id":"a/b","execution_target":"game"});
    assert_eq!(
        code(&s, "POST", "/api/tasks", task).await,
        StatusCode::CREATED
    );
    task::set_status(&s, "played", "ready").unwrap();
    let claim_game = json!({"worker":"gamer","execution_target":"game"});
    let (code_, claim) = request(&s, "POST", "/worker/claim", claim_game.clone()).await;
    assert_eq!(code_, StatusCode::OK);
    assert_eq!(claim["task"]["id"], "played");
    let report = json!({"claim_id":claim["claim_id"],"outcome":"done","report_markdown":"ok"});
    assert_eq!(
        code(&s, "POST", "/worker/report", report).await,
        StatusCode::OK
    );

    let (code_, deleted) = request(&s, "DELETE", &format!("{targets}/game"), json!(null)).await;
    assert_eq!(code_, StatusCode::OK);
    assert_eq!(
        deleted,
        json!({"labels":["forge","field","lab_2"],"default":"forge"})
    );
    assert_eq!(request(&s, "GET", targets, json!(null)).await.1, deleted);
    for (label, expected) in [
        ("game", StatusCode::NOT_FOUND),
        ("forge", StatusCode::CONFLICT),
        ("Bad", StatusCode::BAD_REQUEST),
    ] {
        let path = format!("{targets}/{label}");
        assert_eq!(
            code(&s, "DELETE", &path, json!(null)).await,
            expected,
            "{label}"
        );
    }
    let again = json!({"title":"again","execution_target":"game"});
    assert_eq!(
        code(&s, "POST", "/api/tasks", again).await,
        StatusCode::BAD_REQUEST
    );
    let assign = json!({"execution_target":"game"});
    assert_eq!(
        code(&s, "PATCH", "/api/tasks/played", assign).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        code(&s, "POST", "/worker/claim", claim_game).await,
        StatusCode::BAD_REQUEST
    );
    let (code_, card) = request(&s, "GET", "/api/tasks/played", json!(null)).await;
    assert_eq!(code_, StatusCode::OK);
    assert_eq!(card["execution_target"], "game");
    assert_eq!(card["execution_target_configured"], false);
    let filter = "/api/tasks?status=done&execution_target=game";
    assert_eq!(
        request(&s, "GET", filter, json!(null)).await.1[0]["id"],
        "played"
    );
    drop(s);
    let s = state(root.path(), Some(path.to_str().unwrap())).unwrap();
    assert_eq!(request(&s, "GET", targets, json!(null)).await.1, deleted);
}

#[tokio::test]
async fn emptied_definitions_are_not_imported_again() {
    let root = tempfile::tempdir().unwrap();
    let s = state(root.path(), None).unwrap();
    let (code, created) = request(
        &s,
        "POST",
        "/api/execution-targets",
        json!({"label":"solo"}),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED);
    assert_eq!(created, json!({"labels":["solo"],"default":null}));
    let (code, deleted) = request(&s, "DELETE", "/api/execution-targets/solo", json!(null)).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(deleted, json!({"labels":[],"default":null}));
    drop(s);
    let path = root.path().join("targets.yaml");
    std::fs::write(&path, "labels: [forge]\ndefault: forge\n").unwrap();
    let s = state(root.path(), Some(path.to_str().unwrap())).unwrap();
    assert_eq!(
        request(&s, "GET", "/api/execution-targets", json!(null))
            .await
            .1,
        deleted
    );
}
