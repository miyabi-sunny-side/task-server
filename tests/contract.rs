//! Contract tests for the shipped task-server HTTP API.
//!
//! Everything here goes through the router, so the tests fail if the JSON
//! surface, the authorization rules, or the sqlite truth drift apart.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use time::macros::datetime;
use tower::ServiceExt;

use task_server::db::Db;
use task_server::{AppState, SharedClock};

const USER: &str = "miyabi";
const ORIGIN: &str = "https://task-server.test";
const CSRF: &str = "test-csrf";
const CAPABILITY: &str = "test-capability";
const PRODUCT: &str = "sunny-side/task-server";
const KEEPER: &str = "sunny-side/workers";

fn state_for(db: &Arc<Db>) -> AppState {
    AppState::for_test()
        .with_db(db.clone())
        .with_clock(Arc::new(SharedClock::at(
            datetime!(2026-08-15 10:00:00 UTC),
        )))
}

fn file_backed_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("tempdir");
    let db = Arc::new(Db::open(dir.path().join("state/task-server.db")).expect("open db"));
    let state = state_for(&db);
    (dir, state)
}

/// A file-backed state whose clock the test drives by hand, so lease expiry can
/// be crossed without sleeping.
fn clocked_state(ttl_secs: u64) -> (TempDir, AppState, SharedClock) {
    let dir = TempDir::new().expect("tempdir");
    let db = Arc::new(Db::open(dir.path().join("state/task-server.db")).expect("open db"));
    let clock = SharedClock::at(datetime!(2026-08-15 10:00:00 UTC));
    let state = AppState::for_test()
        .with_db(db)
        .with_clock(Arc::new(clock.clone()))
        .with_ttl(ttl_secs);
    (dir, state, clock)
}

/// Reopen the same database file behind a fresh `AppState`, proving the truth
/// lives in sqlite and not in the process.
fn reopen(dir: &TempDir, previous: AppState) -> AppState {
    drop(previous);
    let db = Arc::new(Db::open(dir.path().join("state/task-server.db")).expect("reopen db"));
    state_for(&db)
}

async fn send(state: &AppState, request: Request<Body>) -> (StatusCode, Value) {
    let response = task_server::app(state.clone())
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// The response without the JSON assumption: status, `content-type`, and the
/// raw bytes as text. `send` turns anything unparseable into `Value::Null`,
/// which is exactly what a test about non-JSON refusals must not do.
async fn send_raw(state: &AppState, request: Request<Body>) -> (StatusCode, String, String) {
    let response = task_server::app(state.clone())
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn request(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
}

fn read(uri: &str) -> Request<Body> {
    request("GET", uri)
        .header("x-auth-user", USER)
        .body(Body::empty())
        .expect("read request")
}

fn human(method: &str, uri: &str, body: &Value) -> Request<Body> {
    request(method, uri)
        .header("x-auth-user", USER)
        .header("origin", ORIGIN)
        .header("x-csrf-token", CSRF)
        .body(Body::from(body.to_string()))
        .expect("human request")
}

fn worker(uri: &str, body: &Value) -> Request<Body> {
    request("POST", uri)
        .header("x-worker-capability", CAPABILITY)
        .body(Body::from(body.to_string()))
        .expect("worker request")
}

async fn put_product(state: &AppState, id: &str, releases: bool) -> Value {
    let (status, value) = send(
        state,
        human(
            "PUT",
            &format!("/api/products/{id}"),
            &json!({
                "repository": format!("https://github.com/{id}.git"),
                "description": "under test",
                "releases": releases,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "put product: {value}");
    value
}

async fn create_task(state: &AppState, body: &Value) -> Value {
    let (status, value) = send(state, human("POST", "/api/tasks", body)).await;
    assert_eq!(status, StatusCode::CREATED, "create task: {value}");
    value
}

async fn set_status(state: &AppState, id: &str, to: &str) -> Value {
    let (status, value) = send(
        state,
        human(
            "POST",
            &format!("/api/tasks/{id}/status"),
            &json!({ "status": to }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "set status {to}: {value}");
    assert_eq!(value["status"], to);
    value
}

/// Register a task for `PRODUCT` at `priority` and promote it to `ready`.
async fn ready_task(state: &AppState, id: &str, priority: i64) {
    create_task(
        state,
        &json!({
            "id": id,
            "title": format!("task {id}"),
            "product_id": PRODUCT,
            "priority": priority,
        }),
    )
    .await;
    set_status(state, id, "ready").await;
}

/// A listing row carries the card's identity and none of its body.
fn assert_summary_shape(summary: &Value) {
    for field in [
        "id",
        "title",
        "status",
        "kind",
        "product_id",
        "priority",
        "updated_at",
    ] {
        assert!(
            summary.get(field).is_some(),
            "summary must carry {field}: {summary}"
        );
    }
    assert!(
        summary.get("body").is_none(),
        "the summary is not a full card: {summary}"
    );
}

fn ids_of(listing: &Value) -> Vec<&str> {
    listing
        .as_array()
        .expect("json array")
        .iter()
        .map(|item| item["id"].as_str().expect("id"))
        .collect()
}

fn report_body(claim_id: &str, commit_sha: &str) -> Value {
    json!({
        "claim_id": claim_id,
        "commit_sha": commit_sha,
        "verification": "cargo test",
    })
}

fn report_with_checks(claim_id: &str, commit_sha: &str, checks: &Value) -> Value {
    json!({
        "claim_id": claim_id,
        "commit_sha": commit_sha,
        "verification": "cargo test",
        "checks": checks,
    })
}

fn green_checks() -> Value {
    json!([
        {"name": "cargo test", "exit_code": 0},
        {"name": "cargo clippy", "exit_code": 0},
    ])
}

fn claim_id_of(lease: &Value) -> String {
    lease["claim_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .expect("claim_id")
        .to_owned()
}

async fn get_task(state: &AppState, id: &str) -> (StatusCode, Value) {
    send(state, read(&format!("/api/tasks/{id}"))).await
}

async fn post_status(state: &AppState, id: &str, to: &str) -> (StatusCode, Value) {
    send(
        state,
        human(
            "POST",
            &format!("/api/tasks/{id}/status"),
            &json!({"status": to}),
        ),
    )
    .await
}

async fn post_report(state: &AppState, body: &Value) -> (StatusCode, Value) {
    send(state, worker("/worker/report", body)).await
}

async fn post_merge(state: &AppState, task_id: &str) -> (StatusCode, Value) {
    send(
        state,
        human("POST", "/api/merges", &json!({"task_id": task_id})),
    )
    .await
}

async fn post_release(state: &AppState, product_id: &str, tag: &str) -> (StatusCode, Value) {
    send(
        state,
        human(
            "POST",
            "/api/releases",
            &json!({"product_id": product_id, "tag": tag}),
        ),
    )
    .await
}

/// Lease the next task and assert the scheduler handed out the expected one.
async fn claim_next(state: &AppState, expected_id: &str) -> String {
    let (status, lease) = send(state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "claim: {lease}");
    assert_eq!(lease["id"], expected_id, "unexpected lease: {lease}");
    claim_id_of(&lease)
}

/// Claim a ready task and report a commit against the lease.
async fn work_to_done(state: &AppState, id: &str, commit_sha: &str) -> Value {
    let claim_id = claim_next(state, id).await;
    let (status, done) = post_report(state, &report_body(&claim_id, commit_sha)).await;
    assert_eq!(status, StatusCode::OK, "report {id}: {done}");
    assert_eq!(done["status"], "done");
    done
}

/// Claim an issued merge and land it with checks that all passed.
async fn land_merge(state: &AppState, merge_id: &str, commit_sha: &str) -> Value {
    let claim_id = claim_next(state, merge_id).await;
    let (status, landed) = post_report(
        state,
        &report_with_checks(&claim_id, commit_sha, &green_checks()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "land {merge_id}: {landed}");
    assert_eq!(landed["status"], "done");
    landed
}

/// Drive one task the whole way to `merged` through the control plane.
async fn drive_to_merged(state: &AppState, id: &str, product_id: &str, commit_sha: &str) -> String {
    create_task(
        state,
        &json!({"id": id, "title": format!("task {id}"), "product_id": product_id}),
    )
    .await;
    set_status(state, id, "ready").await;
    work_to_done(state, id, commit_sha).await;

    let (status, merge) = post_merge(state, id).await;
    assert_eq!(status, StatusCode::CREATED, "issue merge for {id}: {merge}");
    let merge_id = merge["id"].as_str().expect("merge id").to_owned();
    land_merge(state, &merge_id, commit_sha).await;

    let (status, target) = get_task(state, id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(target["status"], "merged", "{id} must be merged: {target}");
    merge_id
}

/// A refused merge report leaves both rows exactly where they were.
async fn assert_merge_did_not_land(state: &AppState, merge_id: &str, target_id: &str) {
    let (status, merge) = send(state, read(&format!("/api/tasks/{merge_id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        merge["status"], "wip",
        "the merge must still be leased: {merge}"
    );
    let (status, target) = send(state, read(&format!("/api/tasks/{target_id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        target["status"], "done",
        "the target must not have moved: {target}"
    );
}

async fn control(state: &AppState) -> Value {
    let (status, value) = send(state, read("/api/control")).await;
    assert_eq!(status, StatusCode::OK, "control plane: {value}");
    value
}

fn transitions(card: &Value) -> Vec<String> {
    card["available_transitions"]
        .as_array()
        .expect("available_transitions array")
        .iter()
        .map(|value| value.as_str().expect("status string").to_owned())
        .collect()
}

#[tokio::test]
async fn http_alone_drives_a_task_from_creation_to_release() {
    let (dir, state) = file_backed_state();

    let product = put_product(&state, PRODUCT, true).await;
    assert_eq!(product["id"], PRODUCT);
    assert_eq!(product["releases"], true);

    let created = create_task(
        &state,
        &json!({
            "id": "t-cutover",
            "title": "drive the cutover",
            "body": "本文は CJK を含む",
            "product_id": PRODUCT,
            "priority": 5,
        }),
    )
    .await;
    assert_eq!(created["status"], "draft");
    assert_eq!(created["kind"], "normal");
    assert_eq!(created["priority"], 5);
    assert_eq!(created["body"], "本文は CJK を含む");
    assert!(transitions(&created).contains(&"ready".to_owned()));
    assert!(
        !transitions(&created).contains(&"released".to_owned()),
        "a draft cannot jump to released"
    );

    // The truth is the database file, not this process.
    let state = reopen(&dir, state);
    let (status, survived) = send(&state, read("/api/tasks/t-cutover")).await;
    assert_eq!(status, StatusCode::OK, "task must survive a reopen");
    assert_eq!(survived["title"], "drive the cutover");

    set_status(&state, "t-cutover", "ready").await;

    let (status, lease) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "claim: {lease}");
    assert_eq!(lease["id"], "t-cutover");
    assert_eq!(lease["status"], "wip");
    assert_eq!(lease["claimed_by"], "grok");
    assert_eq!(
        lease["branch"], "task/t-cutover",
        "the branch name is derived from the task id"
    );
    let claim_id = claim_id_of(&lease);

    let (status, reported) = post_report(&state, &report_body(&claim_id, "abc1234")).await;
    assert_eq!(status, StatusCode::OK, "report: {reported}");
    assert_eq!(reported["status"], "done");
    assert_eq!(reported["commit_sha"], "abc1234");
    assert_eq!(reported["verification"], "cargo test");

    // `merged` is the control plane's to grant, never a human status change.
    let (status, refused) = post_status(&state, "t-cutover", "merged").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "merged is not a human transition: {refused}"
    );

    let (status, merge) = post_merge(&state, "t-cutover").await;
    assert_eq!(status, StatusCode::CREATED, "issue merge: {merge}");
    let merge_id = merge["id"].as_str().expect("merge id").to_owned();
    assert_eq!(merge["kind"], "instant:merge");
    assert_eq!(merge["status"], "ready");
    assert_eq!(merge["merge_target_task_id"], "t-cutover");
    assert_eq!(merge["branch"], "task/t-cutover");

    land_merge(&state, &merge_id, "abc1234").await;

    let (status, merged) = get_task(&state, "t-cutover").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(merged["status"], "merged", "a green merge lands: {merged}");
    assert!(
        !transitions(&merged).contains(&"released".to_owned()),
        "released is granted by the release API, not by a status change: {merged}"
    );

    let (status, released) = post_release(&state, PRODUCT, "v0.2.0").await;
    assert_eq!(status, StatusCode::OK, "release: {released}");
    assert_eq!(released["product_id"], PRODUCT);
    assert_eq!(released["tag"], "v0.2.0");
    assert_eq!(ids_of(&released["released"]), ["t-cutover"]);

    let state = reopen(&dir, state);
    let (status, final_card) = get_task(&state, "t-cutover").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(final_card["status"], "released");
    assert_eq!(final_card["release_tag"], "v0.2.0");
    assert_eq!(final_card["branch"], "task/t-cutover");
    assert!(
        transitions(&final_card).is_empty(),
        "released is terminal: {final_card}"
    );
}

#[tokio::test]
async fn merge_candidates_are_done_tasks_and_are_issued_once() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    for id in ["t-a-done", "t-b-wip", "t-c-ready", "t-d-draft"] {
        create_task(
            &state,
            &json!({"id": id, "title": format!("task {id}"), "product_id": PRODUCT}),
        )
        .await;
    }
    for id in ["t-a-done", "t-b-wip", "t-c-ready"] {
        set_status(&state, id, "ready").await;
    }

    work_to_done(&state, "t-a-done", "abc1234").await;
    // Left leased on purpose: a wip task is not a merge candidate.
    claim_next(&state, "t-b-wip").await;

    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["mergeable"]),
        ["t-a-done"],
        "only a done task is mergeable: {plane}"
    );
    assert_eq!(
        ids_of(&plane["pending_merges"]),
        Vec::<&str>::new(),
        "nothing is in flight yet: {plane}"
    );

    let (status, merge) = post_merge(&state, "t-a-done").await;
    assert_eq!(status, StatusCode::CREATED, "issue merge: {merge}");
    let merge_id = merge["id"].as_str().expect("merge id").to_owned();
    assert_eq!(merge["kind"], "instant:merge");
    assert_eq!(merge["status"], "ready", "a merge is claimable at once");
    assert_eq!(merge["merge_target_task_id"], "t-a-done");
    assert_eq!(
        merge["product_id"], PRODUCT,
        "the merge inherits the target's product"
    );
    assert_eq!(
        merge["branch"], "task/t-a-done",
        "the merge inherits the target's branch"
    );
    assert_eq!(
        merge["commit_sha"], "abc1234",
        "the merge inherits the target's commit"
    );

    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["mergeable"]),
        Vec::<&str>::new(),
        "a task with a live merge is no longer a candidate: {plane}"
    );
    assert_eq!(ids_of(&plane["pending_merges"]), [merge_id.as_str()]);

    let (status, listing) = send(&state, read("/api/tasks")).await;
    assert_eq!(status, StatusCode::OK);
    let before = listing.as_array().expect("array").len();

    let (status, again) = post_merge(&state, "t-a-done").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a second merge for the same target conflicts: {again}"
    );
    assert_eq!(again["code"], "conflict", "{again}");

    let (status, listing) = send(&state, read("/api/tasks")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        listing.as_array().expect("array").len(),
        before,
        "a refused merge must not create a task: {listing}"
    );

    for (id, expected) in [
        ("t-c-ready", StatusCode::BAD_REQUEST),
        ("t-missing", StatusCode::NOT_FOUND),
    ] {
        let (status, refused) = post_merge(&state, id).await;
        assert_eq!(status, expected, "merge {id}: {refused}");
    }
}

#[tokio::test]
async fn a_merge_only_lands_when_every_check_passed() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    create_task(
        &state,
        &json!({"id": "t-land", "title": "land me", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-land", "ready").await;
    let done = work_to_done(&state, "t-land", "abc1234").await;
    assert!(
        !transitions(&done).contains(&"merged".to_owned()),
        "a human cannot press merged: {done}"
    );

    let (status, merge) = post_merge(&state, "t-land").await;
    assert_eq!(status, StatusCode::CREATED, "issue merge: {merge}");
    let merge_id = merge["id"].as_str().expect("merge id").to_owned();
    let claim_id = claim_next(&state, &merge_id).await;

    // (a) A merge without evidence and (b) a merge with one red check are both
    // refused, and neither leaves a trace.
    let red = json!([
        {"name": "cargo test", "exit_code": 101},
        {"name": "cargo clippy", "exit_code": 0},
    ]);
    for (body, why) in [
        (report_body(&claim_id, "abc1234"), "a report without checks"),
        (
            report_with_checks(&claim_id, "abc1234", &red),
            "a report with a red check",
        ),
    ] {
        let (status, refused) = post_report(&state, &body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{why} must be refused: {refused}"
        );
        assert_merge_did_not_land(&state, &merge_id, "t-land").await;
    }

    // (c) All green: the merge finishes and the target lands in one step. The
    // lease the refusals rolled back is still the live one.
    let (status, landed) = post_report(
        &state,
        &report_with_checks(&claim_id, "abc1234", &green_checks()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a green report lands: {landed}");
    assert_eq!(landed["status"], "done");
    assert_eq!(landed["checks"][0]["name"], "cargo test");
    assert_eq!(landed["checks"][0]["exit_code"], 0);

    let (status, target) = get_task(&state, "t-land").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(target["status"], "merged", "the target lands: {target}");
    for forbidden in ["merged", "released"] {
        assert!(
            !transitions(&target).contains(&forbidden.to_owned()),
            "{forbidden} must not be offered to a human: {target}"
        );
        let (status, refused) = post_status(&state, "t-land", forbidden).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "status {forbidden} must be refused: {refused}"
        );
        assert!(
            refused["error"]
                .as_str()
                .is_some_and(|message| message.contains("control plane")),
            "the refusal must name the control plane: {refused}"
        );
        assert_eq!(
            refused["code"], "invalid",
            "every error body carries a machine readable code: {refused}"
        );
    }

    let (status, still) = get_task(&state, "t-land").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        still["status"], "merged",
        "refusals change nothing: {still}"
    );
}

/// A landed merge is not a pass forever: the evidence rule applies to every
/// report, so a repeat without green checks is still a 400 and moves nothing.
#[tokio::test]
async fn a_landed_merge_still_refuses_a_report_without_green_checks() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    create_task(
        &state,
        &json!({"id": "t-again", "title": "land me twice", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-again", "ready").await;
    work_to_done(&state, "t-again", "abc1234").await;

    let (status, merge) = post_merge(&state, "t-again").await;
    assert_eq!(status, StatusCode::CREATED, "issue merge: {merge}");
    let merge_id = merge["id"].as_str().expect("merge id").to_owned();
    let claim_id = claim_next(&state, &merge_id).await;

    let (status, landed) = post_report(
        &state,
        &report_with_checks(&claim_id, "abc1234", &green_checks()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a green report lands: {landed}");

    let red = json!([
        {"name": "cargo test", "exit_code": 101},
        {"name": "cargo clippy", "exit_code": 0},
    ]);
    for (body, why) in [
        (report_body(&claim_id, "abc1234"), "a repeat without checks"),
        (
            report_with_checks(&claim_id, "abc1234", &red),
            "a repeat with a red check",
        ),
    ] {
        let (status, refused) = post_report(&state, &body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{why} must be refused even after the merge landed: {refused}"
        );

        let (status, merge) = get_task(&state, &merge_id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(merge["status"], "done", "{why} moved the merge: {merge}");
        assert_eq!(
            merge["checks"],
            green_checks(),
            "{why} overwrote the evidence: {merge}"
        );
        let (status, target) = get_task(&state, "t-again").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            target["status"], "merged",
            "{why} moved the target: {target}"
        );
    }

    // The gate-satisfying repeat is still idempotent.
    let (status, again) = post_report(
        &state,
        &report_with_checks(&claim_id, "abc1234", &green_checks()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a green repeat is accepted: {again}"
    );
    assert_eq!(again["status"], "done");
    let (status, target) = get_task(&state, "t-again").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(target["status"], "merged", "the target stays landed");
}

/// Cancelling a merge frees its target, and the retry has to be reachable under
/// an id of its own that is still a single URL segment.
#[tokio::test]
async fn a_cancelled_merge_can_be_issued_again() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    create_task(
        &state,
        &json!({"id": "t-retry", "title": "retry me", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-retry", "ready").await;
    work_to_done(&state, "t-retry", "abc1234").await;

    let (status, first) = post_merge(&state, "t-retry").await;
    assert_eq!(status, StatusCode::CREATED, "issue merge: {first}");
    let first_id = first["id"].as_str().expect("merge id").to_owned();

    let (status, again) = post_merge(&state, "t-retry").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a live merge still blocks a second issue: {again}"
    );

    let (status, cancelled) = post_status(&state, &first_id, "cancelled").await;
    assert_eq!(status, StatusCode::OK, "cancel the merge: {cancelled}");
    assert_eq!(cancelled["status"], "cancelled");

    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["mergeable"]),
        ["t-retry"],
        "a cancelled merge frees its target: {plane}"
    );

    let (status, retry) = post_merge(&state, "t-retry").await;
    assert_eq!(status, StatusCode::CREATED, "reissue merge: {retry}");
    let retry_id = retry["id"].as_str().expect("merge id").to_owned();
    assert_ne!(retry_id, first_id, "the retry needs an id of its own");
    assert!(
        !retry_id.contains('/'),
        "a task id is one path segment: {retry_id}"
    );
    assert_eq!(retry["kind"], "instant:merge");
    assert_eq!(retry["status"], "ready");
    assert_eq!(retry["merge_target_task_id"], "t-retry");
    assert_eq!(retry["product_id"], PRODUCT);
    assert_eq!(retry["branch"], "task/t-retry");
    assert_eq!(retry["commit_sha"], "abc1234");

    // The retry is addressable and claimable exactly like the first attempt.
    let (status, card) = get_task(&state, &retry_id).await;
    assert_eq!(status, StatusCode::OK, "the retry is readable: {card}");
    assert_eq!(card["id"], retry_id.as_str());
    let (status, still) = get_task(&state, &first_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        still["status"], "cancelled",
        "the cancelled attempt stays on the record: {still}"
    );

    land_merge(&state, &retry_id, "abc1234").await;
    let (status, target) = get_task(&state, "t-retry").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(target["status"], "merged", "the retry lands: {target}");
}

#[tokio::test]
async fn release_stamps_every_merged_task_of_a_releasing_product() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    put_product(&state, KEEPER, false).await;

    drive_to_merged(&state, "t-rel-1", PRODUCT, "aaa1111").await;
    drive_to_merged(&state, "t-rel-2", PRODUCT, "bbb2222").await;
    drive_to_merged(&state, "t-keep-1", KEEPER, "ccc3333").await;

    // A done task of the same product is not part of a release.
    create_task(
        &state,
        &json!({"id": "t-open", "title": "not merged yet", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-open", "ready").await;
    work_to_done(&state, "t-open", "ddd4444").await;

    let plane = control(&state).await;
    assert_eq!(
        plane["releasable"],
        json!([{"product_id": PRODUCT, "task_count": 2}]),
        "only a releasing product accumulates a release: {plane}"
    );

    for (product_id, tag, expected, why) in [
        (
            KEEPER,
            "v9",
            StatusCode::CONFLICT,
            "a product that does not release",
        ),
        (PRODUCT, "  ", StatusCode::BAD_REQUEST, "a blank tag"),
        (
            "sunny-side/missing",
            "v1",
            StatusCode::NOT_FOUND,
            "an unknown product",
        ),
    ] {
        let (status, refused) = post_release(&state, product_id, tag).await;
        assert_eq!(status, expected, "{why} must be refused: {refused}");
    }

    let (status, released) = post_release(&state, PRODUCT, "v0.2.0").await;
    assert_eq!(status, StatusCode::OK, "release: {released}");
    assert_eq!(released["product_id"], PRODUCT);
    assert_eq!(released["tag"], "v0.2.0");
    assert_eq!(ids_of(&released["released"]), ["t-rel-1", "t-rel-2"]);

    for id in ["t-rel-1", "t-rel-2"] {
        let (status, card) = get_task(&state, id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(card["status"], "released", "{id}: {card}");
        assert_eq!(card["release_tag"], "v0.2.0", "{id}: {card}");
    }

    let (status, keeper) = get_task(&state, "t-keep-1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        keeper["status"], "merged",
        "another product is untouched: {keeper}"
    );
    assert_eq!(keeper["release_tag"], Value::Null);

    let (status, open) = get_task(&state, "t-open").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        open["status"], "done",
        "a done task is not released: {open}"
    );
    assert_eq!(open["release_tag"], Value::Null);

    let plane = control(&state).await;
    assert_eq!(
        plane["releasable"],
        json!([]),
        "the release emptied the queue: {plane}"
    );

    let (status, empty) = post_release(&state, PRODUCT, "v0.2.1").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a release with nothing merged conflicts: {empty}"
    );

    for id in ["t-rel-1", "t-rel-2"] {
        let (status, card) = get_task(&state, id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(card["release_tag"], "v0.2.0", "{id} keeps its tag: {card}");
    }
}

#[tokio::test]
async fn claim_prefers_instant_merge_and_listing_hides_released() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    // An instant:merge task exists only because the control plane issued one,
    // so the queue this test schedules against is built the way production
    // builds it: finished work, then POST /api/merges.
    ready_task(&state, "t-merged", 0).await;
    work_to_done(&state, "t-merged", "abc1234").await;

    let (status, merge) = post_merge(&state, "t-merged").await;
    assert_eq!(status, StatusCode::CREATED, "issue merge: {merge}");
    let merge_id = merge["id"].as_str().expect("merge id").to_owned();
    assert_eq!(merge["kind"], "instant:merge");
    assert_eq!(
        merge["priority"], 0,
        "the merge inherits the target's priority, so it outranks on kind alone: {merge}"
    );

    ready_task(&state, "t-normal", 100).await;

    let (status, lease) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        lease["id"],
        merge_id.as_str(),
        "instant:merge outranks a higher priority normal task: {lease}"
    );
    let claim_id = lease["claim_id"].as_str().expect("claim_id").to_owned();

    let (status, rejected) = send(
        &state,
        worker("/worker/report", &report_body("not-the-lease", "abc1234")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a stale claim_id must conflict: {rejected}"
    );
    assert_eq!(rejected["code"], "claim_mismatch", "{rejected}");

    // A merge attempt a human abandons is dropped by hand, and the lease dies
    // with it: the report that was live a moment ago now lands nothing.
    set_status(&state, &merge_id, "dropped").await;
    let (status, abandoned) = post_report(
        &state,
        &report_with_checks(&claim_id, "abc1234", &green_checks()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a dropped merge lands nothing: {abandoned}"
    );
    let (status, target) = get_task(&state, "t-merged").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        target["status"], "done",
        "an abandoned merge leaves its target where it was: {target}"
    );

    // The target then takes the whole control-plane path to `released`.
    let (status, retry) = post_merge(&state, "t-merged").await;
    assert_eq!(status, StatusCode::CREATED, "reissue merge: {retry}");
    let retry_id = retry["id"].as_str().expect("merge id").to_owned();
    land_merge(&state, &retry_id, "abc1234").await;
    let (status, released) = post_release(&state, PRODUCT, "v0.2.0").await;
    assert_eq!(status, StatusCode::OK, "release: {released}");

    let (status, listing) = send(&state, read("/api/tasks")).await;
    assert_eq!(status, StatusCode::OK);
    let mut listed = ids_of(&listing);
    listed.sort_unstable();
    let mut expected = vec!["t-normal", merge_id.as_str(), retry_id.as_str()];
    expected.sort_unstable();
    assert_eq!(listed, expected, "the default listing hides released");
    let summary = listing
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "t-normal")
        .expect("t-normal summary");
    assert_summary_shape(summary);

    let (status, released) = send(&state, read("/api/tasks?status=released")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids_of(&released), ["t-merged"]);

    let (status, _) = send(&state, read("/api/tasks?status=not-a-status")).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unknown status must be 400"
    );
}

/// Registration files ordinary work and nothing else. A hand-made
/// `instant:merge` task would be a merge with no target: claimed ahead of every
/// other task, impossible to report, and so a standing block on the queue. The
/// refusal is explicit, never a silently ignored field.
#[tokio::test]
async fn creating_an_instant_merge_task_by_hand_is_refused() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    let (status, refused) = send(
        &state,
        human(
            "POST",
            "/api/tasks",
            &json!({
                "id": "t-forged",
                "title": "forge a merge",
                "product_id": PRODUCT,
                "kind": "instant:merge",
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "only the control plane issues a merge: {refused}"
    );
    assert_eq!(refused["code"], "invalid", "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .is_some_and(|message| message.contains("/api/merges")),
        "the refusal must point at the control plane: {refused}"
    );

    let (status, missing) = get_task(&state, "t-forged").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a refused creation writes no row: {missing}"
    );

    // The kind that does exist is still accepted, spelled out or left out.
    for (id, body) in [
        (
            "t-explicit",
            json!({"id": "t-explicit", "title": "normal", "product_id": PRODUCT, "kind": "normal"}),
        ),
        (
            "t-default",
            json!({"id": "t-default", "title": "normal", "product_id": PRODUCT}),
        ),
    ] {
        let created = create_task(&state, &body).await;
        assert_eq!(created["id"], id);
        assert_eq!(created["kind"], "normal", "{created}");
    }
}

#[tokio::test]
async fn mutation_requires_identity_and_csrf_while_worker_requires_capability() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    let body = json!({
        "id": "t-auth",
        "title": "authorized",
        "product_id": PRODUCT,
    });

    let (status, created) = send(&state, human("POST", "/api/tasks", &body)).await;
    assert_eq!(status, StatusCode::CREATED, "full headers must pass");
    assert_eq!(created["id"], "t-auth");

    let denied = json!({
        "id": "t-denied",
        "title": "denied",
        "product_id": PRODUCT,
    });

    let no_identity = request("POST", "/api/tasks")
        .header("origin", ORIGIN)
        .header("x-csrf-token", CSRF)
        .body(Body::from(denied.to_string()))
        .unwrap();
    let (status, denied_body) = send(&state, no_identity).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "identity is required");
    assert_eq!(denied_body["code"], "unauthorized", "{denied_body}");

    let no_csrf = request("POST", "/api/tasks")
        .header("x-auth-user", USER)
        .header("origin", ORIGIN)
        .body(Body::from(denied.to_string()))
        .unwrap();
    let (status, _) = send(&state, no_csrf).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "CSRF token is required");

    let capability_only = request("POST", "/api/tasks")
        .header("x-worker-capability", CAPABILITY)
        .body(Body::from(denied.to_string()))
        .unwrap();
    let (status, _) = send(&state, capability_only).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "worker capability is not a human identity"
    );

    let (status, missing) = send(&state, read("/api/tasks/t-denied")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no refused request may have written: {missing}"
    );
    assert_eq!(missing["code"], "not_found", "{missing}");

    let no_capability = request("POST", "/worker/claim")
        .body(Body::from(json!({"worker": "grok"}).to_string()))
        .unwrap();
    let (status, _) = send(&state, no_capability).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "worker routes need a capability"
    );

    set_status(&state, "t-auth", "ready").await;
    let (status, lease) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "capability claims: {lease}");
    assert_eq!(lease["id"], "t-auth");
}

#[tokio::test]
async fn patch_edits_attributes_without_touching_the_workflow() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    create_task(
        &state,
        &json!({"id": "t-edit", "title": "before", "product_id": PRODUCT}),
    )
    .await;

    let (status, patched) = send(
        &state,
        human(
            "PATCH",
            "/api/tasks/t-edit",
            &json!({"title": "after", "priority": 7}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "patch: {patched}");
    assert_eq!(patched["title"], "after");
    assert_eq!(patched["priority"], 7);
    assert_eq!(patched["status"], "draft");

    let (status, _) = send(
        &state,
        human("PATCH", "/api/tasks/t-edit", &json!({"title": "  "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a blank title is refused");

    let (status, _) = send(
        &state,
        human(
            "POST",
            "/api/tasks/t-edit/status",
            &json!({"status": "wip"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "draft cannot jump straight to wip"
    );
}

/// Registering a task is not the moment to know the catalogue: an agent may
/// file work for a product nobody has entered yet. Promoting it to `ready` is,
/// and the refusal has to say which product is missing and carry a code an
/// automated client can branch on.
#[tokio::test]
async fn an_uncatalogued_product_blocks_ready_but_not_registration() {
    const UNLISTED: &str = "sunny-side/unlisted";

    let (_dir, state) = file_backed_state();

    let created = create_task(
        &state,
        &json!({
            "id": "t-unlisted",
            "title": "work for a product nobody entered",
            "product_id": UNLISTED,
        }),
    )
    .await;
    assert_eq!(created["status"], "draft");
    assert_eq!(created["product_id"], UNLISTED);
    assert!(
        transitions(&created).contains(&"ready".to_owned()),
        "the table still allows ready; the gate refuses at the transition: {created}"
    );

    let (status, refused) = post_status(&state, "t-unlisted", "ready").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an uncatalogued product must not be promoted: {refused}"
    );
    assert_eq!(
        refused["code"], "product_not_catalogued",
        "the refusal must be machine readable: {refused}"
    );
    assert!(
        refused["error"]
            .as_str()
            .is_some_and(|message| message.contains(UNLISTED)),
        "the refusal must name the product: {refused}"
    );

    let (status, still) = get_task(&state, "t-unlisted").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        still["status"], "draft",
        "a refused promotion changes nothing: {still}"
    );

    // A task with no product at all is refused under its own code.
    create_task(
        &state,
        &json!({"id": "t-orphan", "title": "work for no product"}),
    )
    .await;
    let (status, orphan) = post_status(&state, "t-orphan", "ready").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a task without a product must not be promoted: {orphan}"
    );
    assert_eq!(orphan["code"], "product_required", "{orphan}");

    // Entering the product in the catalogue is the whole remedy.
    put_product(&state, UNLISTED, true).await;
    set_status(&state, "t-unlisted", "ready").await;

    let (status, lease) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "claim: {lease}");
    assert_eq!(
        lease["id"], "t-unlisted",
        "a catalogued task is handed to a worker: {lease}"
    );
}

#[tokio::test]
async fn products_are_listed_and_read_over_http() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    put_product(&state, "sunny-side/workers", false).await;

    let (status, listing) = send(&state, read("/api/products")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        ids_of(&listing),
        ["sunny-side/task-server", "sunny-side/workers"]
    );

    let (status, one) = send(&state, read("/api/products/sunny-side/workers")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["releases"], false);

    let (status, _) = send(&state, read("/api/products/sunny-side/missing")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(
        &state,
        human(
            "PUT",
            "/api/products/not-an-org-repo",
            &json!({"repository": "https://example.test/x.git"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "ids must be org/repo");
}

#[tokio::test]
async fn session_and_health_stay_available() {
    let (_dir, state) = file_backed_state();

    let (status, session) = send(&state, read("/api/session")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["user"], USER);
    assert_eq!(session["csrf_token"], CSRF);

    let anonymous = Request::builder()
        .uri("/api/session")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&state, anonymous).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// A worker that dies mid-task must not strand it: once the lease it holds is
/// past `claim_expires_at`, the next claim takes the task over with a fresh
/// `claim_id`, and the abandoned lease can no longer report.
#[tokio::test]
async fn an_expired_lease_is_reclaimed_and_the_abandoned_lease_conflicts() {
    let (_dir, state, clock) = clocked_state(60);
    put_product(&state, PRODUCT, true).await;
    create_task(
        &state,
        &json!({"id": "t-lease", "title": "abandoned midway", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-lease", "ready").await;

    let (status, first) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "first claim: {first}");
    assert_eq!(first["claimed_by"], "grok");
    assert_eq!(first["claim_expires_at"], "2026-08-15T10:01:00Z");
    let abandoned = first["claim_id"].as_str().expect("claim_id").to_owned();

    // A lease that is still alive belongs to its holder.
    clock.advance_secs(59);
    let (status, idle) = send(&state, worker("/worker/claim", &json!({"worker": "codex"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        idle,
        json!({"status": "no-work"}),
        "a live lease must not be stolen: {idle}"
    );

    clock.advance_secs(2);
    let (status, retaken) =
        send(&state, worker("/worker/claim", &json!({"worker": "codex"}))).await;
    assert_eq!(status, StatusCode::OK, "expired lease must be reclaimed");
    assert_eq!(retaken["id"], "t-lease");
    assert_eq!(retaken["status"], "wip");
    assert_eq!(retaken["claimed_by"], "codex");
    assert_eq!(retaken["claimed_at"], "2026-08-15T10:01:01Z");
    assert_eq!(retaken["claim_expires_at"], "2026-08-15T10:02:01Z");
    let fresh = retaken["claim_id"].as_str().expect("claim_id").to_owned();
    assert_ne!(fresh, abandoned, "a reclaim must issue a new claim_id");

    let (status, refused) = send(
        &state,
        worker("/worker/report", &report_body(&abandoned, "abc1234")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the abandoned lease must not report: {refused}"
    );

    let (status, done) = send(
        &state,
        worker("/worker/report", &report_body(&fresh, "def5678")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the live lease reports: {done}");
    assert_eq!(done["status"], "done");
    assert_eq!(done["commit_sha"], "def5678");
}

/// The no-work answer is a success with an exact body, not an error and not an
/// empty response: workers poll on it.
#[tokio::test]
async fn a_claim_with_nothing_to_hand_out_answers_no_work() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    let (status, empty) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "an idle claim is not an error");
    assert_eq!(
        empty,
        json!({"status": "no-work"}),
        "no-work body must be exactly this shape: {empty}"
    );

    create_task(
        &state,
        &json!({"id": "t-only", "title": "the only work", "product_id": PRODUCT}),
    )
    .await;

    // A draft is not claimable either.
    let (status, drafted) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(drafted, json!({"status": "no-work"}), "{drafted}");

    set_status(&state, "t-only", "ready").await;
    let (status, lease) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lease["id"], "t-only");

    let (status, exhausted) =
        send(&state, worker("/worker/claim", &json!({"worker": "codex"}))).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "everything claimed is still not an error"
    );
    assert_eq!(
        exhausted,
        json!({"status": "no-work"}),
        "no-work body must be exactly this shape: {exhausted}"
    );
}

/// The CSRF token is checked by value rather than by presence, and a refused
/// mutation must leave nothing behind.
#[tokio::test]
async fn mutations_refuse_a_wrong_csrf_token_without_writing() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    let denied = |id: &str| {
        json!({
            "id": id,
            "title": "must not exist",
            "product_id": PRODUCT,
        })
    };

    // A present but wrong CSRF token is refused: presence alone proves nothing.
    let wrong_csrf = request("POST", "/api/tasks")
        .header("x-auth-user", USER)
        .header("origin", ORIGIN)
        .header("x-csrf-token", "not-the-csrf-token")
        .body(Body::from(denied("t-wrong-csrf").to_string()))
        .unwrap();
    let (status, _) = send(&state, wrong_csrf).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a wrong CSRF token is refused"
    );

    // The same checks guard every human mutation, not just task creation.
    let wrong_csrf_status = request("POST", "/api/tasks/t-only/status")
        .header("x-auth-user", USER)
        .header("origin", ORIGIN)
        .header("x-csrf-token", "not-the-csrf-token")
        .body(Body::from(json!({"status": "ready"}).to_string()))
        .unwrap();
    let (status, _) = send(&state, wrong_csrf_status).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a wrong CSRF token is refused on status changes too"
    );

    let (status, leaked) = send(&state, read("/api/tasks/t-wrong-csrf")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a refused mutation must not have written: {leaked}"
    );
}

/// Which identities and origins reach this server is settled at the ingress —
/// nginx on the LAN, Tailscale Serve on the tailnet — so the server takes the
/// name it is handed and asks nothing about `Origin`. The CSRF token is what
/// remains, because it is the one thing a cross-site page cannot produce.
#[tokio::test]
async fn any_ingress_identity_writes_with_the_csrf_token_alone() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    let body = |id: &str| {
        json!({
            "id": id,
            "title": "written by whoever the ingress named",
            "product_id": PRODUCT,
        })
    };

    // A name no allowlist would have carried, over an Origin no allowlist would
    // have carried either.
    let stranger = request("POST", "/api/tasks")
        .header("x-auth-user", "someone-else")
        .header("origin", "https://evil.example")
        .header("x-csrf-token", CSRF)
        .body(Body::from(body("t-stranger").to_string()))
        .unwrap();
    let (status, created) = send(&state, stranger).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["id"], "t-stranger");

    // The tailnet header names people the LAN never sees, and no Origin arrives
    // from a client that is not a browser.
    let tailnet = request("POST", "/api/tasks")
        .header("tailscale-user-login", "someone@example.test")
        .header("x-csrf-token", CSRF)
        .body(Body::from(body("t-tailnet").to_string()))
        .unwrap();
    let (status, created) = send(&state, tailnet).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["id"], "t-tailnet");

    // Both writes are real, and reads answer the same way.
    for id in ["t-stranger", "t-tailnet"] {
        let stored = request("GET", &format!("/api/tasks/{id}"))
            .header("x-auth-user", "a-third-name")
            .body(Body::empty())
            .unwrap();
        let (status, card) = send(&state, stored).await;
        assert_eq!(status, StatusCode::OK, "{card}");
        assert_eq!(card["id"], id);
    }

    // The session echoes back whatever name arrived, since that is now the
    // whole of what the server knows about who is asking.
    let session = request("GET", "/api/session")
        .header("x-auth-user", "someone-else")
        .body(Body::empty())
        .unwrap();
    let (status, who) = send(&state, session).await;
    assert_eq!(status, StatusCode::OK, "{who}");
    assert_eq!(who["user"], "someone-else");
}

#[test]
fn production_startup_is_fail_closed_without_secrets() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("task-server.db");

    let Err(err) = AppState::from_vars(|key| match key {
        "TASK_SERVER_ENV" => Some("production".into()),
        _ => None,
    }) else {
        panic!("production without secrets must fail");
    };
    let message = err.to_string();
    assert!(
        message.contains("required") || message.contains("WORKER_CAPABILITY"),
        "missing production secrets must fail closed: {message}"
    );

    let ok = AppState::from_vars(|key| match key {
        "TASK_SERVER_ENV" => Some("production".into()),
        "WORKER_CAPABILITY" => Some("secret-cap".into()),
        // The MCP admin endpoint opens task CRUD to an agent, so production
        // demands its own capability for it too.
        "MCP_CAPABILITY" => Some("secret-mcp-cap".into()),
        "APP_CSRF_TOKEN" => Some("secret-csrf".into()),
        "APP_DB_PATH" => Some(db_path.to_string_lossy().into_owned()),
        _ => None,
    })
    .expect("production with secrets");
    assert!(ok.dev_identity.is_none());
    assert_eq!(ok.worker_capability, "secret-cap");
    assert_eq!(ok.mcp_capability, "secret-mcp-cap");
    assert!(db_path.is_file(), "APP_DB_PATH must be opened at startup");
}

/// A path no API route claims is our own refusal, so it answers in the shape
/// every other refusal of ours uses: 404 JSON with the stable `not_found` slug.
/// It must not leak the SPA fallback and must not answer in prose.
#[tokio::test]
async fn an_unknown_api_path_is_refused_as_json_with_a_stable_code() {
    let (_dir, state) = file_backed_state();

    let (status, content_type, text) = send_raw(
        &state,
        request("GET", "/api/nowhere")
            .header("x-auth-user", USER)
            .body(Body::empty())
            .expect("unknown api request"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    assert!(
        content_type.starts_with("application/json"),
        "an unknown API path answers JSON, got {content_type}: {text}"
    );
    let body: Value = serde_json::from_str(&text).expect("json body");
    assert_eq!(body["code"], "not_found", "{body}");
    assert!(
        body["error"].as_str().is_some_and(|it| !it.is_empty()),
        "the message is prose next to the slug, never instead of it: {body}"
    );

    // A POST to an unknown API path is the same refusal, not a 405 and not the
    // client's index.html.
    let (status, content_type, text) = send_raw(
        &state,
        human("POST", "/api/nowhere", &json!({"anything": true})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    assert!(
        content_type.starts_with("application/json"),
        "got {content_type}: {text}"
    );
    let body: Value = serde_json::from_str(&text).expect("json body");
    assert_eq!(body["code"], "not_found", "{body}");
}

/// A request body that is not JSON never reaches our code: axum's own extractor
/// rejects it before the handler runs, in plain text and without a `code`. That
/// is the framework's contract, not ours, and this test pins what it actually
/// does so the documented promise stays true to it.
#[tokio::test]
async fn a_malformed_request_body_is_rejected_by_the_framework_in_plain_text() {
    let (_dir, state) = file_backed_state();

    let (status, content_type, text) = send_raw(
        &state,
        request("POST", "/api/tasks")
            .header("x-auth-user", USER)
            .header("origin", ORIGIN)
            .header("x-csrf-token", CSRF)
            .body(Body::from("{\"id\": \"t-broken\", "))
            .expect("malformed request"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a syntactically broken body is a 400: {text}"
    );
    assert!(
        content_type.starts_with("text/plain"),
        "the framework answers in plain text, got {content_type}: {text}"
    );
    assert!(
        serde_json::from_str::<Value>(&text).is_err(),
        "the rejection body is not JSON: {text}"
    );
    assert!(
        text.contains("Failed to parse the request body as JSON"),
        "the rejection names the parse failure: {text}"
    );

    // Nothing was written, because nothing of ours ever ran.
    let (status, missing) = send(&state, read("/api/tasks/t-broken")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a rejected body must not have written: {missing}"
    );
}
