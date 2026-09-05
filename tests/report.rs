use serde_json::json;
use std::sync::Arc;
use task_server::{AppState, SharedClock, ledger::Store, task};
use time::macros::datetime;

fn setup() -> (tempfile::TempDir, AppState, SharedClock, serde_json::Value) {
    let dir = tempfile::tempdir().unwrap();
    let clock = SharedClock::at(datetime!(2026-09-05 00:00 UTC));
    let state = AppState::new(Store::open(dir.path()).unwrap())
        .with_clock(Arc::new(clock.clone()))
        .with_ttl(10);
    state
        .store
        .put("products", "a/b", json!({"id":"a/b"}))
        .unwrap();
    task::create(
        &state,
        json!({"id":"t","title":"test","body":"instructions","product_id":"a/b"}),
    )
    .unwrap();
    task::set_status(&state, "t", "ready").unwrap();
    let claim = task::claim(&state, "worker").unwrap().unwrap();
    (dir, state, clock, claim)
}

#[test]
fn report_stores_one_raw_document_and_references_without_inventing_verification() {
    let (dir, state, _, claim) = setup();
    let raw = format!(
        "# Result\n\n{}\nUnverified idea remains open.\n",
        "原文".repeat(4000)
    );
    let payload = json!({"claim_id":claim["claim_id"],"outcome":"done","report_markdown":raw,"commit_sha":"abc","milestones":[{"name":"implemented"}],"checks":[]});
    let record = task::report(&state, payload.clone()).unwrap();
    assert_eq!(record["status"], "done");
    let runs = state.store.list("runs").unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run["body"], raw);
    assert_eq!(record["report_id"], run["id"]);
    assert_eq!(record["milestones"][0]["report_id"], run["id"]);
    assert_eq!(record["milestones"].as_array().unwrap().len(), 1);
    assert!(record["verification"].is_null());
    assert_eq!(run["claim_id"], claim["claim_id"]);
    assert_eq!(run["commit_sha"], "abc");
    assert_eq!(task::report(&state, payload.clone()).unwrap(), record);
    let mut changed = payload;
    changed["report_markdown"] = json!("different");
    assert!(task::report(&state, changed).is_err());
    assert_eq!(state.store.list("runs").unwrap().len(), 1);
    let task_file = std::fs::read_to_string(dir.path().join("tasks/t.md")).unwrap();
    assert!(!task_file.contains("Unverified idea"));
    assert_eq!(record["body"], "instructions");
}

#[test]
fn accepted_report_recovers_after_task_write_failure_and_lease_expiry() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, state, clock, claim) = setup();
    let tasks = dir.path().join("tasks");
    std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o555)).unwrap();
    let payload = json!({"claim_id":claim["claim_id"],"outcome":"done","report_markdown":"# preserved\n","commit_sha":"abc"});
    let failure = task::report(&state, payload.clone());
    std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(failure.is_err());
    assert_eq!(state.store.get("tasks", "t").unwrap()["status"], "wip");
    assert_eq!(state.store.list("runs").unwrap().len(), 1);
    clock.advance_secs(11);
    // Reading after an interrupted write replays the accepted intent before expiring leases.
    assert_eq!(task::card(&state, "t").unwrap()["status"], "done");
    assert_eq!(task::report(&state, payload).unwrap()["status"], "done");
    assert_eq!(state.store.list("runs").unwrap().len(), 1);
}

#[test]
fn old_report_retry_after_another_claim_does_not_duplicate_or_revert_progress() {
    let (_dir, state, _, claim) = setup();
    let first = json!({"claim_id":claim["claim_id"],"outcome":"blocked","report_markdown":"remaining work","commit_sha":"abc","milestones":[{"name":"verified"}]});
    let record = task::report(&state, first.clone()).unwrap();
    let first_id = record["report_id"].clone();
    task::set_status(&state, "t", "ready").unwrap();
    let claim = task::claim(&state, "second").unwrap().unwrap();
    task::report(&state,json!({"claim_id":claim["claim_id"],"outcome":"done","report_markdown":"finished","commit_sha":"def"})).unwrap();
    let record = task::report(&state, first).unwrap();
    assert_eq!(record["status"], "done");
    assert_eq!(record["commit_sha"], "def");
    assert_eq!(record["milestone_history"][0]["report_id"], first_id);
    assert_eq!(state.store.list("runs").unwrap().len(), 2);
    task::delete(&state, "t").unwrap();
    assert_eq!(state.store.list("runs").unwrap().len(), 2);
}

#[test]
fn invalid_new_report_has_no_durable_effect() {
    let (_dir, state, _, claim) = setup();
    for extra in [
        json!({"report_markdown":""}),
        json!({"report_markdown":"result","milestones":[{"name":"imaginary"}]}),
        json!({"report_markdown":"result","checks":[{"name":"test","exit_code":1}]}),
    ] {
        let mut payload = extra;
        payload["claim_id"] = claim["claim_id"].clone();
        payload["outcome"] = json!("done");
        assert!(task::report(&state, payload).is_err());
        assert!(state.store.list("runs").unwrap().is_empty());
        assert_eq!(state.store.get("tasks", "t").unwrap()["status"], "wip");
    }
}

#[test]
fn old_completion_prose_remains_readable_without_becoming_the_new_stop_reason() {
    let (_dir, state, clock, claim) = setup();
    task::report(&state,json!({"claim_id":claim["claim_id"],"outcome":"blocked","summary":"old summary","verification":"old reason","checks":["old evidence"]})).unwrap();
    task::set_status(&state, "t", "ready").unwrap();
    let claim = task::claim(&state, "next").unwrap().unwrap();
    let report = task::report(
        &state,
        json!({"claim_id":claim["claim_id"],"outcome":"blocked","report_markdown":"new reason"}),
    )
    .unwrap();
    assert_eq!(report["legacy_completion"][0]["verification"], "old reason");
    assert_eq!(report["legacy_completion"][0]["checks"][0], "old evidence");
    assert!(report["verification"].is_null());
    task::set_status(&state, "t", "ready").unwrap();
    task::claim(&state, "interrupted").unwrap().unwrap();
    clock.advance_secs(11);
    let task = task::card(&state, "t").unwrap();
    assert_eq!(task["verification"], "interrupted: execution lease expired");
}

#[test]
fn legacy_report_after_new_report_selects_current_legacy_result() {
    let (_dir, state, _, claim) = setup();
    let first=task::report(&state,json!({"claim_id":claim["claim_id"],"outcome":"blocked","report_markdown":"earlier original"})).unwrap();
    task::set_status(&state, "t", "ready").unwrap();
    let claim = task::claim(&state, "legacy").unwrap().unwrap();
    let current=task::report(&state,json!({"claim_id":claim["claim_id"],"outcome":"done","summary":"latest legacy result","verification":"latest legacy evidence"})).unwrap();
    assert!(current["report_id"].is_null());
    assert_eq!(current["report_ids"][0], first["report_id"]);
    assert_eq!(current["summary"], "latest legacy result");
    assert_eq!(
        state.store.list("runs").unwrap()[0]["body"],
        "earlier original"
    );
}

#[test]
fn legacy_mutations_cannot_invent_verified_evidence_with_a_missing_report() {
    let (_dir, state, _, claim) = setup();
    let result = task::report(
        &state,
        json!({"claim_id":claim["claim_id"],"outcome":"done","milestones":[{"name":"verified","report_id":999}]}),
    );
    assert!(result.is_err());
    task::set_status(&state, "t", "blocked").unwrap();
    assert!(
        task::patch(
            &state,
            "t",
            json!({"milestones":[{"name":"verified","report_id":999}]})
        )
        .is_err()
    );
    assert_eq!(
        state.store.get("tasks", "t").unwrap()["milestones"],
        json!([])
    );
}
