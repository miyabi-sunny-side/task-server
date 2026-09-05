use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;
use task_server::{AppState, SharedClock, ledger::Store, task};
use time::macros::datetime;
use tower::ServiceExt;

fn state(path: &std::path::Path, clock: &SharedClock) -> AppState {
    AppState::new(Store::open(path).unwrap())
        .with_clock(Arc::new(clock.clone()))
        .with_ttl(10)
}
fn seed(s: &AppState) {
    s.store.put("products", "a/b", json!({"id":"a/b"})).unwrap();
    task::create(s, json!({"id":"t","title":"checkpoint","product_id":"a/b"})).unwrap();
}
fn claim(s: &AppState) -> String {
    task::set_status(s, "t", "ready").unwrap();
    task::claim(s, "test").unwrap().unwrap()["claim_id"]
        .as_str()
        .unwrap()
        .into()
}
async fn request(s: &AppState, method: &str, body: Value) -> (StatusCode, Value) {
    let response = task_server::app(s.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri("/worker/tasks/t/checkpoint")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}
async fn patch(
    s: &AppState,
    claim: &str,
    revision: u64,
    set: Value,
    delete: Value,
) -> (StatusCode, Value) {
    request(
        s,
        "POST",
        json!({"claim_id":claim,"expected_revision":revision,"set":set,"delete_keys":delete}),
    )
    .await
}
#[tokio::test]
async fn partial_updates_delete_and_restart_keep_values_and_formal_state() {
    let dir = tempfile::tempdir().unwrap();
    let clock = SharedClock::at(datetime!(2026-09-05 00:00 UTC));
    let s = state(dir.path(), &clock);
    seed(&s);
    assert_eq!(
        request(&s, "GET", Value::Null).await.1["checkpoints"],
        json!([])
    );
    let id = claim(&s);
    let before = s.store.get("tasks", "t").unwrap();
    let (status, first) = patch(&s, &id, 0, json!({"ci_url":"https://example.test/runs/1","next_step":"wait_ci","nullable":null,"context":{"branch":"task/t"}}), json!([])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["revision"], 1);
    clock.advance_secs(1);
    let (status, second) = patch(
        &s,
        &id,
        1,
        json!({"next_step":"merge"}),
        json!(["context", "absent"]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        second["values"],
        json!({"ci_url":"https://example.test/runs/1","next_step":"merge","nullable":null})
    );
    assert_ne!(first["updated_at"], second["updated_at"]);
    let mut after = s.store.get("tasks", "t").unwrap();
    after["execution_checkpoints"] = before["execution_checkpoints"].clone();
    assert_eq!(
        after, before,
        "checkpoint does not mutate lifecycle, lease or official evidence"
    );
    drop(s);
    let s = state(dir.path(), &clock);
    let saved = request(&s, "GET", Value::Null).await.1;
    assert_eq!(saved["checkpoints"][0], second);
    assert_eq!(saved["active_claim_id"], id);
}
#[tokio::test]
async fn expiry_new_execution_and_stale_writes_are_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let clock = SharedClock::at(datetime!(2026-09-05 00:00 UTC));
    let s = state(dir.path(), &clock);
    seed(&s);
    let old = claim(&s);
    assert_eq!(
        patch(&s, &old, 0, json!({"next_step":"wait_ci"}), json!([]))
            .await
            .0,
        StatusCode::OK
    );
    clock.advance_secs(11);
    let saved = request(&s, "GET", Value::Null).await.1;
    assert!(saved["active_claim_id"].is_null());
    assert_eq!(saved["checkpoints"][0]["values"]["next_step"], "wait_ci");
    assert_eq!(
        patch(&s, &old, 1, json!({"next_step":"wrong"}), json!([]))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let new = claim(&s);
    assert_ne!(old, new);
    assert_eq!(
        patch(&s, &old, 1, json!({"next_step":"wrong"}), json!([]))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let saved = request(&s, "GET", Value::Null).await.1;
    assert_eq!(saved["checkpoints"].as_array().unwrap().len(), 2);
    assert_eq!(saved["checkpoints"][1]["execution_id"], new);
    assert_eq!(saved["checkpoints"][1]["values"], json!({}));
    assert_eq!(saved["checkpoints"][0]["values"]["next_step"], "wait_ci");
}
#[tokio::test]
async fn concurrent_revision_prevents_lost_updates_and_allows_reapply() {
    let dir = tempfile::tempdir().unwrap();
    let clock = SharedClock::at(datetime!(2026-09-05 00:00 UTC));
    let s = state(dir.path(), &clock);
    seed(&s);
    let id = claim(&s);
    let (one, two) = tokio::join!(
        patch(&s, &id, 0, json!({"branch":"task/t"}), json!([])),
        patch(&s, &id, 0, json!({"next_step":"review"}), json!([]))
    );
    assert_eq!(
        [one.0, two.0]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        [one.0, two.0]
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        1
    );
    let missing = if one.0 == StatusCode::OK {
        json!({"next_step":"review"})
    } else {
        json!({"branch":"task/t"})
    };
    assert_eq!(
        patch(&s, &id, 1, missing, json!([])).await.1["values"],
        json!({"branch":"task/t","next_step":"review"})
    );
}
#[tokio::test]
async fn invalid_patches_are_atomic_and_cannot_write_through_other_claims() {
    let dir = tempfile::tempdir().unwrap();
    let clock = SharedClock::at(datetime!(2026-09-05 00:00 UTC));
    let s = state(dir.path(), &clock);
    seed(&s);
    let id = claim(&s);
    task::create(&s, json!({"id":"other","title":"other","product_id":"a/b"})).unwrap();
    task::set_status(&s, "other", "ready").unwrap();
    let other = task::claim(&s, "other").unwrap().unwrap()["claim_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let bytes = std::fs::read(dir.path().join("tasks/t.md")).unwrap();
    assert_eq!(
        patch(&s, &other, 0, json!({"x":1}), json!([])).await.0,
        StatusCode::CONFLICT
    );
    for body in [
        json!({"claim_id":id,"set":{"x":1}}),
        json!({"claim_id":id,"expected_revision":0,"set":[]}),
        json!({"claim_id":id,"expected_revision":0,"delete_keys":[2]}),
        json!({"claim_id":id,"expected_revision":0,"set":{"x":1},"delete_keys":["x"]}),
        json!({"claim_id":id,"expected_revision":0,"set":{"":1}}),
        json!({"claim_id":id,"expected_revision":0,"set":{"x":"a".repeat(32769)}}),
        json!({"claim_id":id,"expected_revision":0,"status":"done"}),
    ] {
        assert_eq!(request(&s, "POST", body).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read(dir.path().join("tasks/t.md")).unwrap(), bytes);
    }
}

#[tokio::test]
async fn old_server_live_claim_gets_revision_zero_without_reclaim_or_read_write() {
    let dir = tempfile::tempdir().unwrap();
    let clock = SharedClock::at(datetime!(2026-09-05 00:00 UTC));
    let s = state(dir.path(), &clock);
    seed(&s);
    let id = claim(&s);
    s.store
        .update("tasks", "t", |t| {
            t.as_object_mut().unwrap().remove("execution_checkpoints");
            Ok(())
        })
        .unwrap();
    let before = std::fs::read(dir.path().join("tasks/t.md")).unwrap();
    let read = request(&s, "GET", Value::Null).await.1;
    assert_eq!(read["checkpoints"][0]["execution_id"], id);
    assert_eq!(read["checkpoints"][0]["revision"], 0);
    assert_eq!(read["checkpoints"][0]["values"], json!({}));
    assert_eq!(
        std::fs::read(dir.path().join("tasks/t.md")).unwrap(),
        before
    );
    assert_eq!(
        patch(&s, &id, 0, json!({"next_step":"review"}), json!([]))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&s, "GET", Value::Null).await.1["checkpoints"][0]["revision"],
        1
    );
}
