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
const STALE_WORKER_CAPABILITY: &str = "old-test-capability";
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
        .body(Body::from(body.to_string()))
        .expect("worker request")
}

async fn assert_worker_routes_ignore_obsolete_capability(state: &AppState) {
    for (route, body, expected) in [
        ("/worker/claim", json!({"worker": "probe"}), StatusCode::OK),
        (
            "/worker/report",
            report_body("no-such-claim", "abc1234"),
            StatusCode::CONFLICT,
        ),
        (
            "/worker/review-report",
            review_report_body("no-such-claim", "abc1234", "approve", "read it"),
            StatusCode::CONFLICT,
        ),
    ] {
        for stale_header in [false, true] {
            let mut probe = request("POST", route);
            if stale_header {
                probe = probe.header("x-worker-capability", STALE_WORKER_CAPABILITY);
            }
            let probe = probe.body(Body::from(body.to_string())).unwrap();
            let (status, response) = send(state, probe).await;
            assert_eq!(
                status, expected,
                "{route} must reach the domain with stale_header={stale_header}: {response}"
            );
        }
    }
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

async fn post_review(state: &AppState, task_id: &str) -> (StatusCode, Value) {
    send(
        state,
        human("POST", "/api/reviews", &json!({"task_id": task_id})),
    )
    .await
}

async fn post_review_report(state: &AppState, body: &Value) -> (StatusCode, Value) {
    send(state, worker("/worker/review-report", body)).await
}

fn review_report_body(claim_id: &str, subject: &str, verdict: &str, findings: &str) -> Value {
    json!({
        "claim_id": claim_id,
        "subject_commit_sha": subject,
        "verdict": verdict,
        "findings": findings,
    })
}

async fn post_merge(state: &AppState, task_id: &str) -> (StatusCode, Value) {
    send(
        state,
        human("POST", "/api/merges", &json!({"task_id": task_id})),
    )
    .await
}

async fn post_release(state: &AppState, product_id: &str) -> (StatusCode, Value) {
    send(
        state,
        human("POST", "/api/releases", &json!({"product_id": product_id})),
    )
    .await
}

/// Claim the release the landing of `product_id`'s work issued, and report the
/// tag it cut, the way a release worker does. Nobody files it: landing is what
/// puts it in front of the worker.
async fn ship_release(state: &AppState, product_id: &str, tag: &str) -> Value {
    let (status, lease) = claim_kind(state, "shipper", &json!(["instant:release"])).await;
    assert_eq!(status, StatusCode::OK, "claim release: {lease}");
    assert_eq!(lease["kind"], "instant:release", "{lease}");
    assert_eq!(lease["product_id"], product_id, "{lease}");
    let claim_id = claim_id_of(&lease);
    let (status, shipped) = post_report(
        state,
        &json!({
            "claim_id": claim_id,
            "commit_sha": "fff0000",
            "verification": "bump-tag",
            "checks": [{"name": "bump-tag", "exit_code": 0}],
            "release_tag": tag,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "ship {product_id}: {shipped}");
    assert_eq!(shipped["status"], "released", "{shipped}");
    assert_eq!(shipped["release_tag"], tag, "{shipped}");
    shipped
}

/// Lease the next task and assert the scheduler handed out the expected one.
async fn claim_next(state: &AppState, expected_id: &str) -> String {
    let (status, lease) = send(state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "claim: {lease}");
    assert_eq!(lease["id"], expected_id, "unexpected lease: {lease}");
    claim_id_of(&lease)
}

/// Lease the next task of one of `kinds`, the way a worker with a role does.
async fn claim_kind(state: &AppState, worker_name: &str, kinds: &Value) -> (StatusCode, Value) {
    send(
        state,
        worker(
            "/worker/claim",
            &json!({"worker": worker_name, "kinds": kinds}),
        ),
    )
    .await
}

/// Claim the review the report of `id` issued, and answer it. Nobody files it:
/// finishing the work is what puts it in front of a reviewer.
async fn answer_review(
    state: &AppState,
    id: &str,
    subject: &str,
    verdict: &str,
    findings: &str,
) -> Value {
    let (status, lease) = claim_kind(state, "sol", &json!(["review"])).await;
    assert_eq!(status, StatusCode::OK, "claim review: {lease}");
    assert_eq!(
        lease["review_target_task_id"], id,
        "the reviewer must get the review of {id}: {lease}"
    );
    let claim_id = claim_id_of(&lease);

    let (status, answered) = post_review_report(
        state,
        &review_report_body(&claim_id, subject, verdict, findings),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "review {id}: {answered}");
    assert_eq!(answered["status"], "done", "a verdict finishes the review");
    answered
}

/// Have a review approve the commit `id` reported, the only way to `approved`.
async fn approve_task(state: &AppState, id: &str, subject: &str) -> Value {
    answer_review(
        state,
        id,
        subject,
        "approve",
        "read the diff, ran the tests",
    )
    .await;
    let (status, card) = get_task(state, id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(card["status"], "approved", "{id} must be approved: {card}");
    card
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

/// The review the report of `id` issued, read back through the API.
async fn issued_review(state: &AppState, id: &str) -> Value {
    let (status, review) = get_task(state, &format!("review:{id}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the report of {id} must have issued a review: {review}"
    );
    review
}

/// The merge the approval of `id` issued, read back through the API.
async fn issued_merge(state: &AppState, id: &str) -> Value {
    let (status, merge) = get_task(state, &format!("merge:{id}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the approval of {id} must have issued a merge: {merge}"
    );
    merge
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
    approve_task(state, id, commit_sha).await;

    let merge = issued_merge(state, id).await;
    let merge_id = merge["id"].as_str().expect("merge id").to_owned();
    land_merge(state, &merge_id, commit_sha).await;

    let (status, target) = get_task(state, id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(target["status"], "merged", "{id} must be merged: {target}");
    merge_id
}

/// The landing issued the release and nobody pressed anything: the work is
/// pointed at it, it is in flight, nothing is stranded, and a second issue is
/// refused.
async fn assert_release_was_issued_for(state: &AppState, merged: &Value, release_id: &str) {
    assert_eq!(
        merged["release_task_id"], release_id,
        "the landed work is pointed at its release: {merged}"
    );
    let plane = control(state).await;
    assert_eq!(ids_of(&plane["pending_releases"]), [release_id]);
    assert_eq!(plane["pending_releases"][0]["release_level"], "patch");
    assert_eq!(plane["releasable"], json!([]), "{plane}");
    let product = merged["product_id"].as_str().expect("product id");
    let (status, refused) = post_release(state, product).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a release in flight is not issued twice: {refused}"
    );
}

/// Drive one task of a product that does not release through its landing. The
/// target ends `released` rather than `merged`, so this returns the merge id
/// without asserting the status `drive_to_merged` asserts.
async fn drive_to_merged_or_released(
    state: &AppState,
    id: &str,
    product_id: &str,
    commit_sha: &str,
) -> String {
    create_task(
        state,
        &json!({"id": id, "title": format!("task {id}"), "product_id": product_id}),
    )
    .await;
    set_status(state, id, "ready").await;
    work_to_done(state, id, commit_sha).await;
    approve_task(state, id, commit_sha).await;
    let merge = issued_merge(state, id).await;
    let merge_id = merge["id"].as_str().expect("merge id").to_owned();
    let claim_id = claim_next(state, &merge_id).await;
    let (status, landed) = post_report(
        state,
        &report_with_checks(&claim_id, commit_sha, &green_checks()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "land {merge_id}: {landed}");
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
        target["status"], "approved",
        "the target must not have moved: {target}"
    );
}

/// The control plane once `id` was approved: its merge is on the queue, its
/// review is answered, and there is nothing left for a human to press.
async fn assert_merge_is_in_flight(state: &AppState, id: &str) {
    let plane = control(state).await;
    assert_eq!(
        ids_of(&plane["mergeable"]),
        Vec::<&str>::new(),
        "approved work is already in flight: {plane}"
    );
    assert_eq!(
        ids_of(&plane["pending_merges"]),
        [format!("merge:{id}").as_str()],
        "{plane}"
    );
    assert_eq!(ids_of(&plane["pending_reviews"]), Vec::<&str>::new());
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

    // Nothing reaches the merge queue unread: a review approves the commit
    // that was reported, and only then is the task a merge candidate.
    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["mergeable"]),
        Vec::<&str>::new(),
        "work nobody reviewed is not mergeable: {plane}"
    );
    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["pending_reviews"]),
        ["review:t-cutover"],
        "the report put the work in front of a reviewer: {plane}"
    );
    let approved = approve_task(&state, "t-cutover", "abc1234").await;
    assert_eq!(approved["latest_review"]["verdict"], "approve");

    // The approval issues the merge, so nothing is waiting to be pressed.
    assert_merge_is_in_flight(&state, "t-cutover").await;

    let merge = issued_merge(&state, "t-cutover").await;
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

    assert_release_was_issued_for(&state, &merged, "release:t-cutover").await;
    ship_release(&state, PRODUCT, "v0.2.0").await;

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
async fn merge_candidates_are_approved_tasks_and_are_issued_once() {
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
    // Left leased on purpose: a wip task is not a merge candidate. The filter
    // keeps this claim off the review the report just queued.
    let (status, lease) = claim_kind(&state, "grok", &json!(["normal"])).await;
    assert_eq!(status, StatusCode::OK, "claim: {lease}");
    assert_eq!(lease["id"], "t-b-wip", "unexpected lease: {lease}");

    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["mergeable"]),
        Vec::<&str>::new(),
        "a done task nobody reviewed is not a candidate yet: {plane}"
    );
    approve_task(&state, "t-a-done", "abc1234").await;

    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["mergeable"]),
        Vec::<&str>::new(),
        "the approval already issued the merge: {plane}"
    );

    let merge = issued_merge(&state, "t-a-done").await;
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
    assert!(
        !transitions(&done).contains(&"approved".to_owned()),
        "a human cannot press approved either: {done}"
    );
    approve_task(&state, "t-land", "abc1234").await;

    let merge = issued_merge(&state, "t-land").await;
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
    for forbidden in ["approved", "merged", "released"] {
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
    approve_task(&state, "t-again", "abc1234").await;

    let merge = issued_merge(&state, "t-again").await;
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
    approve_task(&state, "t-retry", "abc1234").await;

    let first = issued_merge(&state, "t-retry").await;
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

/// Two landings of the releasing product, the second while the release the
/// first issued is still open; and one landing of a product that does not
/// release, which ends there. Returns the merge id of that last one.
async fn land_two_behind_one_open_release(state: &AppState) -> String {
    put_product(state, PRODUCT, true).await;
    put_product(state, KEEPER, false).await;
    let keeper_merge = drive_to_merged_or_released(state, "t-keep-1", KEEPER, "ccc3333").await;

    drive_to_merged(state, "t-rel-1", PRODUCT, "aaa1111").await;
    create_task(
        state,
        &json!({
            "id": "t-rel-2",
            "title": "task t-rel-2",
            "product_id": PRODUCT,
            "release_level": "minor",
            // Above the release now waiting, so the kind-less claims the
            // helpers make below take this work and not the release.
            "priority": 5,
        }),
    )
    .await;
    set_status(state, "t-rel-2", "ready").await;
    work_to_done(state, "t-rel-2", "bbb2222").await;
    approve_task(state, "t-rel-2", "bbb2222").await;
    let second = issued_merge(state, "t-rel-2").await;
    land_merge(state, second["id"].as_str().unwrap(), "bbb2222").await;
    keeper_merge
}

#[tokio::test]
async fn landing_issues_one_release_per_product_and_ends_work_that_does_not_release() {
    let (_dir, state) = file_backed_state();
    let keeper_merge = land_two_behind_one_open_release(&state).await;

    // A product that does not release ended at the landing, with no tag.
    for id in ["t-keep-1", &keeper_merge, "review:t-keep-1"] {
        let (status, card) = get_task(&state, id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(card["status"], "released", "{id}: {card}");
        assert_eq!(card["release_tag"], Value::Null, "{id}: {card}");
    }

    // The releasing product got its release at the first landing, and the
    // second landing waited for it.
    let (_, waiting) = get_task(&state, "t-rel-2").await;
    assert_eq!(waiting["status"], "merged");
    assert_eq!(waiting["release_task_id"], Value::Null, "{waiting}");
    let plane = control(&state).await;
    assert_eq!(ids_of(&plane["pending_releases"]), ["release:t-rel-1"]);
    assert_eq!(plane["pending_releases"][0]["status"], "ready");
    assert_eq!(plane["pending_releases"][0]["verification"], Value::Null);
    assert_eq!(
        plane["releasable"],
        json!([]),
        "nothing is stranded: {plane}"
    );

    for (product_id, expected, why) in [
        (
            KEEPER,
            StatusCode::CONFLICT,
            "a product that does not release",
        ),
        (
            PRODUCT,
            StatusCode::CONFLICT,
            "a product with a release in flight",
        ),
        (
            "sunny-side/missing",
            StatusCode::NOT_FOUND,
            "an unknown product",
        ),
    ] {
        let (status, refused) = post_release(&state, product_id).await;
        assert_eq!(status, expected, "{why} must be refused: {refused}");
    }
}

#[tokio::test]
async fn a_release_report_ships_everything_it_carries_and_gathers_the_rest() {
    let (_dir, state) = file_backed_state();
    land_two_behind_one_open_release(&state).await;

    // The claim card carries the level, and a report without a tag is refused.
    let (status, lease) = claim_kind(&state, "shipper", &json!(["instant:release"])).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    assert_eq!(lease["id"], "release:t-rel-1");
    assert_eq!(lease["release_level"], "patch");
    let claim_id = claim_id_of(&lease);
    for tag in [Value::Null, json!("0.2.0"), json!("v0.2")] {
        let (status, refused) = post_report(
            &state,
            &json!({
                "claim_id": claim_id,
                "commit_sha": "fff0000",
                "verification": "bump-tag",
                "checks": [{"name": "bump-tag", "exit_code": 0}],
                "release_tag": tag,
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{tag} is not a release tag: {refused}"
        );
    }
    let (status, shipped) = post_report(
        &state,
        &json!({
            "claim_id": claim_id,
            "commit_sha": "fff0000",
            "verification": "bump-tag",
            "checks": [{"name": "bump-tag", "exit_code": 0}],
            "release_tag": "v0.2.0",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "ship: {shipped}");
    assert_eq!(shipped["status"], "released");
    assert_eq!(shipped["release_tag"], "v0.2.0");

    for id in ["t-rel-1", "review:t-rel-1", "merge:t-rel-1"] {
        let (status, card) = get_task(&state, id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(card["status"], "released", "{id}: {card}");
        assert_eq!(card["release_tag"], "v0.2.0", "{id}: {card}");
    }

    // The report gathered what landed meanwhile: the next release, one level up.
    let plane = control(&state).await;
    assert_eq!(ids_of(&plane["pending_releases"]), ["release:t-rel-2"]);
    assert_eq!(plane["pending_releases"][0]["release_level"], "minor");
    let (_, carried) = get_task(&state, "t-rel-2").await;
    assert_eq!(carried["status"], "merged");
    assert_eq!(carried["release_task_id"], "release:t-rel-2");

    ship_release(&state, PRODUCT, "v0.3.0").await;
    let (_, card) = get_task(&state, "t-rel-2").await;
    assert_eq!(card["status"], "released");
    assert_eq!(card["release_tag"], "v0.3.0");
    let plane = control(&state).await;
    assert_eq!(plane["pending_releases"], json!([]), "{plane}");
    let (status, empty) = post_release(&state, PRODUCT).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a release with nothing merged conflicts: {empty}"
    );
}

/// A blocked release stops with its reason on the row and its work still
/// merged; it is called off and reissued by hand, never restarted.
#[tokio::test]
async fn a_blocked_release_is_called_off_and_reissued_by_hand() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    drive_to_merged(&state, "t-1", PRODUCT, "aaa1111").await;

    let (status, lease) = claim_kind(&state, "shipper", &json!(["instant:release"])).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    let claim_id = claim_id_of(&lease);
    let (status, blocked) = post_report(
        &state,
        &json!({
            "claim_id": claim_id,
            "commit_sha": "aaa1111",
            "verification": "bump-tag: the tag already exists",
            "checks": [{"name": "bump-tag", "exit_code": 1}],
            "outcome": "blocked",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{blocked}");
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(transitions(&blocked), ["cancelled", "dropped"]);

    let plane = control(&state).await;
    assert_eq!(ids_of(&plane["pending_releases"]), ["release:t-1"]);
    assert_eq!(
        plane["pending_releases"][0]["verification"], "bump-tag: the tag already exists",
        "the screen reads the reason off the same payload: {plane}"
    );
    let (_, target) = get_task(&state, "t-1").await;
    assert_eq!(target["status"], "merged", "nothing shipped: {target}");

    let (status, refused) = post_release(&state, PRODUCT).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    let (status, refused) = post_status(&state, "release:t-1", "ready").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["code"], "invalid");

    set_status(&state, "release:t-1", "cancelled").await;
    let plane = control(&state).await;
    assert_eq!(
        plane["releasable"],
        json!([{"product_id": PRODUCT, "task_count": 1}]),
        "work whose release was called off is stranded: {plane}"
    );
    let (status, again) = post_release(&state, PRODUCT).await;
    assert_eq!(status, StatusCode::CREATED, "reissue: {again}");
    assert_eq!(again["id"], "release:t-1~2");
    let (_, target) = get_task(&state, "t-1").await;
    assert_eq!(target["release_task_id"], "release:t-1~2");
    ship_release(&state, PRODUCT, "v0.2.1").await;
}

/// `release_level` is on every task from the moment it is filed, defaults to
/// patch, and refuses anything outside its vocabulary.
#[tokio::test]
async fn release_level_is_filed_with_the_task_and_defaults_to_patch() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    let created = create_task(
        &state,
        &json!({"id": "t-default", "title": "no level said", "product_id": PRODUCT}),
    )
    .await;
    assert_eq!(created["release_level"], "patch", "{created}");

    let created = create_task(
        &state,
        &json!({
            "id": "t-major",
            "title": "a breaking change",
            "product_id": PRODUCT,
            "release_level": "major",
        }),
    )
    .await;
    assert_eq!(created["release_level"], "major", "{created}");

    let (status, refused) = send(
        &state,
        human(
            "POST",
            "/api/tasks",
            &json!({"id": "t-bad", "title": "x", "product_id": PRODUCT, "release_level": "huge"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["code"], "invalid");

    let (status, patched) = send(
        &state,
        human(
            "PATCH",
            "/api/tasks/t-default",
            &json!({"release_level": "minor"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["release_level"], "minor");

    // The subtasks inherit it.
    set_status(&state, "t-default", "ready").await;
    work_to_done(&state, "t-default", "abc1234").await;
    let (_, review) = get_task(&state, "review:t-default").await;
    assert_eq!(review["release_level"], "minor", "{review}");
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
    approve_task(&state, "t-merged", "abc1234").await;

    let merge = issued_merge(&state, "t-merged").await;
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
        target["status"], "approved",
        "an abandoned merge leaves its target where it was: {target}"
    );

    // The target then takes the whole control-plane path to `released`.
    let (status, retry) = post_merge(&state, "t-merged").await;
    assert_eq!(status, StatusCode::CREATED, "reissue merge: {retry}");
    let retry_id = retry["id"].as_str().expect("merge id").to_owned();
    land_merge(&state, &retry_id, "abc1234").await;
    ship_release(&state, PRODUCT, "v0.2.0").await;

    // The review and the landed retry both finished reading or landing
    // `t-merged`, so release carries them to `released` with it, and the
    // default listing hides them the same way it hides `t-merged` itself.
    // The dropped first attempt never finished, so it stays.
    let (status, listing) = send(&state, read("/api/tasks")).await;
    assert_eq!(status, StatusCode::OK);
    let mut listed = ids_of(&listing);
    listed.sort_unstable();
    let mut expected = vec!["t-normal", merge_id.as_str()];
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
    let mut released_ids = ids_of(&released);
    released_ids.sort_unstable();
    let mut expected_released = vec![
        "t-merged",
        "review:t-merged",
        "release:t-merged",
        retry_id.as_str(),
    ];
    expected_released.sort_unstable();
    assert_eq!(released_ids, expected_released);

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
async fn human_mutation_requires_identity_and_csrf_while_worker_routes_need_no_secret() {
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

    let obsolete_worker_header_only = request("POST", "/api/tasks")
        .header("x-worker-capability", STALE_WORKER_CAPABILITY)
        .body(Body::from(denied.to_string()))
        .unwrap();
    let (status, _) = send(&state, obsolete_worker_header_only).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an obsolete worker header is not a human identity"
    );

    let (status, missing) = send(&state, read("/api/tasks/t-denied")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no refused request may have written: {missing}"
    );
    assert_eq!(missing["code"], "not_found", "{missing}");

    assert_worker_routes_ignore_obsolete_capability(&state).await;

    // Issuing a review is a human decision, on the same terms as a merge.
    let review_without_csrf = request("POST", "/api/reviews")
        .header("x-auth-user", USER)
        .body(Body::from(json!({"task_id": "t-auth"}).to_string()))
        .unwrap();
    let (status, _) = send(&state, review_without_csrf).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "issuing a review is a human mutation"
    );

    set_status(&state, "t-auth", "ready").await;
    let (status, lease) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "headerless claim: {lease}");
    assert_eq!(lease["id"], "t-auth");

    let claim_id = claim_id_of(&lease);
    let (status, reported) = send(
        &state,
        worker("/worker/report", &report_body(&claim_id, "abc1234")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "headerless report: {reported}");
    assert_eq!(reported["status"], "done");

    let (status, review) = send(
        &state,
        worker(
            "/worker/claim",
            &json!({"worker": "reviewer", "kinds": ["review"]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "headerless review claim: {review}");
    assert_eq!(review["id"], "review:t-auth");

    let review_claim_id = claim_id_of(&review);
    let (status, reviewed) = send(
        &state,
        worker(
            "/worker/review-report",
            &review_report_body(&review_claim_id, "abc1234", "approve", "read it"),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "headerless review report: {reviewed}"
    );
    assert_eq!(reviewed["status"], "done");
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
    assert_eq!(
        one["archived"], false,
        "whether a product still has a working copy is on the API surface"
    );

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

/// A retry key turns an uncertain claim response back into the same live lease.
/// The receipt and the task move in one transaction, so even concurrent retries
/// consume one task; reusing the key for another request or after expiry is an
/// explicit conflict rather than a chance to take a second task.
#[tokio::test]
async fn an_idempotent_claim_replays_one_live_lease() {
    let (_dir, state, clock) = clocked_state(60);
    put_product(&state, PRODUCT, true).await;
    ready_task(&state, "t-first", 10).await;
    ready_task(&state, "t-second", 0).await;

    let body = || {
        json!({
            "worker": "grok",
            "kinds": ["normal"],
            "idempotency_key": "claim-attempt-1",
        })
    };
    let (left, right) = tokio::join!(
        send(&state, worker("/worker/claim", &body())),
        send(&state, worker("/worker/claim", &body())),
    );
    for (status, lease) in [&left, &right] {
        assert_eq!(*status, StatusCode::OK, "claim retry: {lease}");
        assert_eq!(lease["id"], "t-first", "claim retry: {lease}");
    }
    assert_eq!(left.1["claim_id"], right.1["claim_id"]);

    let (status, second) = send(&state, worker("/worker/claim", &json!({"worker": "codex"}))).await;
    assert_eq!(status, StatusCode::OK, "second task: {second}");
    assert_eq!(second["id"], "t-second", "the retry consumed one task");

    let (status, reused) = send(
        &state,
        worker(
            "/worker/claim",
            &json!({
                "worker": "another-worker",
                "kinds": ["normal"],
                "idempotency_key": "claim-attempt-1",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "key reuse: {reused}");
    assert_eq!(reused["code"], "claim_idempotency_conflict", "{reused}");

    clock.advance_secs(61);
    let (status, expired) = send(&state, worker("/worker/claim", &body())).await;
    assert_eq!(status, StatusCode::CONFLICT, "expired replay: {expired}");
    assert_eq!(expired["code"], "claim_idempotency_conflict", "{expired}");
}

/// The no-work answer is a success with an exact body, not an error and not an
/// empty response: workers poll on it. It has no side effect to remember, so a
/// retry key may claim work that appeared after the empty answer.
#[tokio::test]
async fn a_claim_with_nothing_to_hand_out_answers_no_work() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    let retry = json!({"worker": "grok", "idempotency_key": "idle-attempt"});
    let (status, empty) = send(&state, worker("/worker/claim", &retry)).await;
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
    let (status, lease) = send(&state, worker("/worker/claim", &retry)).await;
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
        message.contains("APP_CSRF_TOKEN"),
        "missing production secrets must fail closed: {message}"
    );

    let ok = AppState::from_vars(|key| match key {
        "TASK_SERVER_ENV" => Some("production".into()),
        "APP_CSRF_TOKEN" => Some("secret-csrf".into()),
        "APP_DB_PATH" => Some(db_path.to_string_lossy().into_owned()),
        _ => None,
    })
    .expect("production with its remaining secret");
    assert!(ok.dev_identity.is_none());
    assert_eq!(ok.csrf_token, "secret-csrf");
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

/// A review is issued against finished work, remembers the commit it was issued
/// for, and is the only open one until it answers. A review that answered is
/// over, so the next round is issued under an id of its own.
#[tokio::test]
async fn a_review_is_issued_once_and_a_finished_one_frees_the_target() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    for id in ["t-read", "t-open"] {
        create_task(
            &state,
            &json!({"id": id, "title": format!("task {id}"), "product_id": PRODUCT}),
        )
        .await;
    }
    set_status(&state, "t-read", "ready").await;
    work_to_done(&state, "t-read", "abc1234").await;

    let review = issued_review(&state, "t-read").await;
    assert_eq!(review["kind"], "review");
    assert_eq!(review["status"], "ready", "a review is claimable at once");
    assert_eq!(review["review_target_task_id"], "t-read");
    assert_eq!(review["product_id"], PRODUCT);
    assert_eq!(review["branch"], "task/t-read");
    assert_eq!(
        review["commit_sha"], "abc1234",
        "the review is issued for the commit the work reported: {review}"
    );
    assert_eq!(review["review_verdict"], Value::Null);

    let (status, again) = post_review(&state, "t-read").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "one open review per target: {again}"
    );
    assert_eq!(again["code"], "conflict", "{again}");

    let (status, refused) = post_review(&state, "t-open").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unfinished work has nothing to review: {refused}"
    );
    let (status, missing) = post_review(&state, "t-missing").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");

    // The verdict finishes this attempt, and the next one is issued freely.
    let (status, lease) = claim_kind(&state, "sol", &json!(["review"])).await;
    assert_eq!(status, StatusCode::OK, "claim review: {lease}");
    assert_eq!(lease["id"], "review:t-read");
    let (status, answered) = post_review_report(
        &state,
        &review_report_body(
            &claim_id_of(&lease),
            "abc1234",
            "request_changes",
            "the empty case is unguarded",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a verdict is a success: {answered}");

    work_to_done(&state, "t-read", "def5678").await;
    let (status, second) = get_task(&state, "review:t-read~2").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a finished attempt must not block the next: {second}"
    );
    assert!(
        !second["id"].as_str().expect("id").contains('/'),
        "a task id is one path segment: {second}"
    );
    assert_eq!(
        second["commit_sha"], "def5678",
        "the second review is of the reworked commit: {second}"
    );
}

/// The status route is the operator's way into the workflow, and a review is
/// the one thing it may not finish: `done` there would record no verdict, say
/// nothing to the parent, and free the target for the next review as though the
/// reading had happened. The refusal has to leave the whole world where it was.
#[tokio::test]
async fn the_status_route_cannot_finish_a_review_without_a_verdict() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    create_task(
        &state,
        &json!({"id": "t-read", "title": "needs a read", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-read", "ready").await;
    work_to_done(&state, "t-read", "abc1234").await;

    issued_review(&state, "t-read").await;
    let (status, lease) = claim_kind(&state, "sol", &json!(["review"])).await;
    assert_eq!(status, StatusCode::OK, "claim review: {lease}");
    assert_eq!(lease["id"], "review:t-read");

    let (status, refused) = post_status(&state, "review:t-read", "done").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a press must not finish a review: {refused}"
    );
    assert_eq!(refused["code"], "invalid", "{refused}");

    let (status, held) = get_task(&state, "review:t-read").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(held["status"], "wip", "a refusal moves no row: {held}");
    assert_eq!(held["review_verdict"], Value::Null, "{held}");
    assert!(
        !transitions(&held).contains(&"done".to_owned()),
        "done is never offered on a review: {held}"
    );

    let (status, parent) = get_task(&state, "t-read").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parent["status"], "done",
        "the parent is untouched: {parent}"
    );
    assert_eq!(
        parent["latest_review"],
        Value::Null,
        "nothing was answered: {parent}"
    );

    let (status, again) = post_review(&state, "t-read").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the refused press must not have freed the one-open-review index: {again}"
    );

    // Calling the review off is a different act, still pressable, and that one
    // does free the target — which is the point of abandoning an attempt.
    let (status, cancelled) = post_status(&state, "review:t-read", "cancelled").await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    let (status, next) = post_review(&state, "t-read").await;
    assert_eq!(status, StatusCode::CREATED, "{next}");
    assert_eq!(next["id"], "review:t-read~2");
}

/// A review that answered is over, and the status route may not raise it.
///
/// `blocked` is the press that mattered: the single-open-review index stops at
/// `done`, `cancelled` and `dropped`, so a finished attempt pushed back to
/// `blocked` would stand in the way of the next review — and from there it walks
/// to `ready`, gets claimed, and reports a second verdict over the first.
#[tokio::test]
async fn the_status_route_cannot_raise_an_answered_review() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    create_task(
        &state,
        &json!({"id": "t-read", "title": "needs a read", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-read", "ready").await;
    work_to_done(&state, "t-read", "abc1234").await;
    answer_review(
        &state,
        "t-read",
        "abc1234",
        "request_changes",
        "the empty case is unguarded",
    )
    .await;

    let (status, answered) = get_task(&state, "review:t-read").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answered["status"], "done");
    assert_eq!(answered["review_verdict"], "request_changes");
    assert_eq!(
        transitions(&answered),
        Vec::<String>::new(),
        "an answered review offers nothing to press: {answered}"
    );

    for to in ["blocked", "cancelled", "dropped", "ready", "wip", "done"] {
        let (status, refused) = post_status(&state, "review:t-read", to).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "pressing {to} on an answered review: {refused}"
        );
        assert_eq!(refused["code"], "invalid", "{refused}");
        let (status, held) = get_task(&state, "review:t-read").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(held, answered, "the refusal of {to} writes nothing: {held}");
    }

    let (status, parent) = get_task(&state, "t-read").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parent["status"], "ready",
        "the verdict the work lives by stands: {parent}"
    );
    assert_eq!(parent["latest_review"]["verdict"], "request_changes");

    let (status, none) = claim_kind(&state, "sol", &json!(["review"])).await;
    assert_eq!(status, StatusCode::OK, "{none}");
    assert_eq!(
        none["status"], "no-work",
        "an answered review cannot be leased again: {none}"
    );

    // And the frozen attempt does not keep the next one out: the report of the
    // rework issues it.
    work_to_done(&state, "t-read", "def5678").await;
    let (status, next) = get_task(&state, "review:t-read~2").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a finished attempt blocks nothing: {next}"
    );
    assert_eq!(next["commit_sha"], "def5678");
}

/// A merge carries the commit it was issued for, and `approved` alone does not
/// say which commit was approved. Work reopened, redone and approved again on
/// another commit must not be landed by the merge that read the old one.
#[tokio::test]
async fn a_merge_issued_for_a_commit_the_work_has_left_behind_lands_nothing() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    create_task(
        &state,
        &json!({"id": "t-move", "title": "moves on", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-move", "ready").await;
    work_to_done(&state, "t-move", "abc1234").await;
    approve_task(&state, "t-move", "abc1234").await;

    let merge = issued_merge(&state, "t-move").await;
    assert_eq!(merge["commit_sha"], "abc1234");

    // The work is taken back, redone on another commit, and approved again.
    set_status(&state, "t-move", "blocked").await;
    set_status(&state, "t-move", "ready").await;
    let (status, lease) = claim_kind(&state, "opus", &json!(["normal"])).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    assert_eq!(lease["id"], "t-move");
    let (status, redone) = post_report(&state, &report_body(&claim_id_of(&lease), "def5678")).await;
    assert_eq!(status, StatusCode::OK, "{redone}");
    approve_task(&state, "t-move", "def5678").await;

    // The stale merge is claimed and reported green: its checks passed and its
    // target is `approved`, so the commit is the only thing that says no.
    let (status, lease) = claim_kind(&state, "luna", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    assert_eq!(lease["id"], "merge:t-move");
    let (status, refused) = post_report(
        &state,
        &report_with_checks(&claim_id_of(&lease), "merge999", &green_checks()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a merge of a commit the work left behind must be refused: {refused}"
    );
    assert_eq!(refused["code"], "merge_subject_changed", "{refused}");

    assert_merge_did_not_land(&state, "merge:t-move", "t-move").await;
    let (_, held) = get_task(&state, "merge:t-move").await;
    assert_eq!(
        held["commit_sha"], "abc1234",
        "the refusal does not take the commit the report carried: {held}"
    );
    assert_eq!(held["verification"], Value::Null, "{held}");
    let (_, target) = get_task(&state, "t-move").await;
    assert_eq!(
        target["commit_sha"], "def5678",
        "and the work stands where it was: {target}"
    );

    // A merge issued for the commit the review approved still lands.
    let (status, cancelled) = post_status(&state, "merge:t-move", "cancelled").await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    let (status, fresh) = post_merge(&state, "t-move").await;
    assert_eq!(status, StatusCode::CREATED, "{fresh}");
    assert_eq!(fresh["id"], "merge:t-move~2");
    assert_eq!(fresh["commit_sha"], "def5678");
    land_merge(&state, "merge:t-move~2", "merge999").await;
    let (status, merged) = get_task(&state, "t-move").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(merged["status"], "merged", "{merged}");
}

/// The round trip a worker lives in: the review hands the work back with its
/// findings, the worker reads them off its own task, reworks, and the next
/// review approves.
#[tokio::test]
async fn a_review_hands_work_back_with_findings_the_worker_can_read() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    create_task(
        &state,
        &json!({"id": "t-fix", "title": "needs a second look", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-fix", "ready").await;
    work_to_done(&state, "t-fix", "abc1234").await;

    let answered = answer_review(
        &state,
        "t-fix",
        "abc1234",
        "request_changes",
        "the empty case is unguarded",
    )
    .await;
    assert_eq!(answered["review_verdict"], "request_changes");
    assert_eq!(answered["verification"], "the empty case is unguarded");

    let (status, sent_back) = get_task(&state, "t-fix").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        sent_back["status"], "ready",
        "work sent back is claimable again: {sent_back}"
    );
    assert_eq!(
        sent_back["latest_review"]["verdict"], "request_changes",
        "the worker reads the verdict off its own task: {sent_back}"
    );
    assert_eq!(
        sent_back["latest_review"]["findings"], "the empty case is unguarded",
        "the findings have to be readable, or the rework is guesswork: {sent_back}"
    );
    assert_eq!(sent_back["latest_review"]["review_task_id"], "review:t-fix");
    assert_eq!(sent_back["latest_review"]["subject_commit_sha"], "abc1234");

    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["mergeable"]),
        Vec::<&str>::new(),
        "work sent back is not on its way to the main line: {plane}"
    );

    // The rework is reviewed again, and the newest verdict is what answers.
    work_to_done(&state, "t-fix", "def5678").await;
    let approved = approve_task(&state, "t-fix", "def5678").await;
    assert_eq!(approved["latest_review"]["verdict"], "approve");
    assert_eq!(
        approved["latest_review"]["review_task_id"],
        "review:t-fix~2"
    );
    assert_eq!(approved["latest_review"]["subject_commit_sha"], "def5678");
    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["pending_merges"]),
        ["merge:t-fix"],
        "the approval put it on the main line's queue: {plane}"
    );
    assert_eq!(
        ids_of(&plane["mergeable"]),
        Vec::<&str>::new(),
        "and nothing is left for a human to press: {plane}"
    );
}

/// The accident the snapshot exists to stop: a reviewer reads one commit, the
/// author lands another while the review is open, and the approval would carry
/// the unread commit onto the main line.
#[tokio::test]
async fn an_approval_of_a_commit_the_work_has_left_behind_is_refused() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    create_task(
        &state,
        &json!({"id": "t-stale", "title": "moving target", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-stale", "ready").await;
    work_to_done(&state, "t-stale", "abc1234").await;

    issued_review(&state, "t-stale").await;
    let (status, lease) = claim_kind(&state, "sol", &json!(["review"])).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    let claim_id = claim_id_of(&lease);

    // Naming a commit this review was not issued for is refused outright.
    let (status, mismatch) = post_review_report(
        &state,
        &review_report_body(&claim_id, "def5678", "approve", "looks good"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{mismatch}");
    assert_eq!(mismatch["code"], "review_subject_mismatch", "{mismatch}");

    // The author takes the task back and finishes it on another commit.
    set_status(&state, "t-stale", "blocked").await;
    set_status(&state, "t-stale", "ready").await;
    work_to_done(&state, "t-stale", "def5678").await;

    let (status, overtaken) = post_review_report(
        &state,
        &review_report_body(&claim_id, "abc1234", "approve", "looked good at the time"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{overtaken}");
    assert_eq!(overtaken["code"], "review_subject_changed", "{overtaken}");

    let (status, parent) = get_task(&state, "t-stale").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parent["status"], "done",
        "a refused approval promotes nothing: {parent}"
    );
    assert_eq!(parent["latest_review"], Value::Null, "{parent}");
    let (status, still_open) = get_task(&state, "review:t-stale").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        still_open["status"], "wip",
        "a refused report writes nothing at all: {still_open}"
    );
    assert_eq!(still_open["review_verdict"], Value::Null);
    assert_eq!(still_open["verification"], Value::Null);

    // A parent a human moved out of `done` is not waiting for a verdict either.
    set_status(&state, "t-stale", "blocked").await;
    let (status, moved) = post_review_report(
        &state,
        &review_report_body(&claim_id, "abc1234", "approve", "read the diff"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{moved}");
    assert_eq!(moved["code"], "review_target_moved", "{moved}");

    let (status, refused) = post_review_report(
        &state,
        &review_report_body(&claim_id, "abc1234", "shrug", "not a verdict"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["code"], "invalid", "{refused}");
}

/// Take `id` all the way to `approved`, which is where its merge is issued.
/// The claim is filtered because the queue now has reviews and merges in it, and
/// this helper is about the ordinary work.
async fn approved_task(state: &AppState, id: &str, product_id: &str, commit_sha: &str) {
    create_task(
        state,
        &json!({"id": id, "title": format!("task {id}"), "product_id": product_id}),
    )
    .await;
    set_status(state, id, "ready").await;
    let (status, lease) = claim_kind(state, "grok", &json!(["normal"])).await;
    assert_eq!(status, StatusCode::OK, "claim {id}: {lease}");
    assert_eq!(lease["id"], id, "unexpected lease: {lease}");
    let (status, done) = post_report(state, &report_body(&claim_id_of(&lease), commit_sha)).await;
    assert_eq!(status, StatusCode::OK, "report {id}: {done}");
    approve_task(state, id, commit_sha).await;
}

fn blocked_report_body(claim_id: &str, commit_sha: &str, reason: &str, checks: &Value) -> Value {
    json!({
        "claim_id": claim_id,
        "commit_sha": commit_sha,
        "verification": reason,
        "checks": checks,
        "outcome": "blocked",
    })
}

/// The control plane answers with what is in flight and with the one thing that
/// should never be there: work that finished and has nobody reading it.
#[tokio::test]
async fn the_control_plane_shows_what_is_in_flight_and_what_lost_its_reader() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    create_task(
        &state,
        &json!({"id": "t-1", "title": "watched", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-1", "ready").await;
    work_to_done(&state, "t-1", "abc1234").await;

    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["pending_reviews"]),
        ["review:t-1"],
        "the report put the work in front of a reviewer: {plane}"
    );
    assert_eq!(
        ids_of(&plane["unreviewed"]),
        Vec::<&str>::new(),
        "so nothing is stranded: {plane}"
    );
    assert_summary_shape(&plane["pending_reviews"][0]);

    // Cancelling the attempt is how that reader is lost, and `done` has no way
    // forward without a verdict — so the window is what makes it visible.
    let (status, cancelled) = post_status(&state, "review:t-1", "cancelled").await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    let plane = control(&state).await;
    assert_eq!(ids_of(&plane["pending_reviews"]), Vec::<&str>::new());
    assert_eq!(
        ids_of(&plane["unreviewed"]),
        ["t-1"],
        "work nobody is reading has to be visible: {plane}"
    );

    // Issuing one by hand is the remedy, and closes the window.
    let (status, second) = post_review(&state, "t-1").await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let plane = control(&state).await;
    assert_eq!(ids_of(&plane["pending_reviews"]), ["review:t-1~2"]);
    assert_eq!(ids_of(&plane["unreviewed"]), Vec::<&str>::new());

    approve_task(&state, "t-1", "abc1234").await;
    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["pending_reviews"]),
        Vec::<&str>::new(),
        "an answered review is not pending: {plane}"
    );
    assert_eq!(ids_of(&plane["pending_merges"]), ["merge:t-1"]);
    assert_eq!(
        ids_of(&plane["mergeable"]),
        Vec::<&str>::new(),
        "and nothing is left for a human to press: {plane}"
    );
}

/// A product's merges go out one at a time, because each one rebases onto the
/// main line the one before it wrote. **Which of them goes first is not
/// promised**, so this reads the answer rather than dictating it. A merge that
/// could not be integrated stops that product until a human calls it off — and
/// stops nobody else's.
#[tokio::test]
async fn a_products_merges_are_handed_out_one_at_a_time_and_a_blocked_one_stops_them() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    put_product(&state, KEEPER, true).await;

    approved_task(&state, "t-a-first", PRODUCT, "aaa1111").await;
    approved_task(&state, "t-b-second", PRODUCT, "bbb2222").await;
    approved_task(&state, "t-z-elsewhere", KEEPER, "ccc3333").await;

    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["pending_merges"]),
        ["merge:t-a-first", "merge:t-b-second", "merge:t-z-elsewhere"],
        "the list is stable — oldest first, ties by id — and only that: {plane}"
    );

    // Whichever of this product's two merges is handed out is the one that runs;
    // the test follows it rather than naming it in advance.
    let (status, lease) = claim_kind(&state, "luna", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    let running = lease["id"].as_str().expect("id").to_owned();
    let subject = lease["commit_sha"].as_str().expect("commit_sha").to_owned();
    let waiting_id = ["merge:t-a-first", "merge:t-b-second"]
        .into_iter()
        .find(|id| *id != running)
        .expect("one of the product's merges runs, the other waits")
        .to_owned();
    assert_ne!(running, "merge:t-z-elsewhere", "{lease}");
    let claim_id = claim_id_of(&lease);

    // Another product is rebasing onto another main line, so its merge runs
    // beside this one.
    let (status, other) = claim_kind(&state, "sol", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{other}");
    assert_eq!(other["id"], "merge:t-z-elsewhere", "{other}");

    let (status, waiting) = claim_kind(&state, "grok", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{waiting}");
    assert_eq!(
        waiting["status"], "no-work",
        "the second merge of a product waits for the first: {waiting}"
    );

    // The head cannot be integrated, and says so. That is a report, not an
    // error: the reason and the checks are kept on the merge.
    let checks = json!([{"name": "git rebase", "exit_code": 1}]);
    let (status, blocked) = post_report(
        &state,
        &blocked_report_body(
            &claim_id,
            &subject,
            "rebase onto main conflicts in src/task.rs",
            &checks,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a blocked report is kept: {blocked}"
    );
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(
        blocked["verification"], "rebase onto main conflicts in src/task.rs",
        "the reason has to be readable: {blocked}"
    );
    assert_eq!(blocked["checks"], checks, "{blocked}");
    assert_eq!(
        blocked["commit_sha"], subject,
        "a blocked merge keeps the subject it was issued for: {blocked}"
    );
    let landed = blocked["merge_target_task_id"]
        .as_str()
        .expect("target")
        .to_owned();
    let (status, target) = get_task(&state, &landed).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        target["status"], "approved",
        "nothing landed, so the target does not move: {target}"
    );

    // Reporting the same thing again is accepted and changes nothing.
    let (status, again) = post_report(
        &state,
        &blocked_report_body(
            &claim_id,
            &subject,
            "rebase onto main conflicts in src/task.rs",
            &checks,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again, blocked, "a repeat writes nothing new: {again}");

    let (status, still_waiting) = claim_kind(&state, "grok", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{still_waiting}");
    assert_eq!(
        still_waiting["status"], "no-work",
        "and the blocked train is still stopped: {still_waiting}"
    );

    // Calling the blocked attempt off is what moves the train.
    let (status, cancelled) = post_status(&state, &running, "cancelled").await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    let (status, moved) = claim_kind(&state, "grok", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["id"], waiting_id, "{moved}");

    // The work the cancelled merge would have landed is a merge candidate
    // again: the reconciliation window is what a human presses on.
    let plane = control(&state).await;
    assert_eq!(ids_of(&plane["mergeable"]), [landed.as_str()], "{plane}");
    let (status, reissued) = post_merge(&state, &landed).await;
    assert_eq!(status, StatusCode::CREATED, "{reissued}");
    assert_eq!(reissued["id"], format!("{running}~2"), "{reissued}");
}

/// A merge that could not be integrated is called off and reissued, never
/// restarted.
///
/// The card offers the two presses that call it off and refuses the one that
/// would hand this very attempt back to a worker, so the screen and the API say
/// the same thing about how a jam clears. The reason it stopped rides along
/// with the queue on `/api/control`, which is what lets a screen name the jam
/// without reading the merge card.
#[tokio::test]
async fn the_status_route_cannot_restart_a_blocked_merge() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    approved_task(&state, "t-a-jam", PRODUCT, "aaa1111").await;
    approved_task(&state, "t-b-behind", PRODUCT, "bbb2222").await;

    let (status, lease) = claim_kind(&state, "luna", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    assert_eq!(lease["id"], "merge:t-a-jam", "{lease}");
    let (status, blocked) = post_report(
        &state,
        &blocked_report_body(
            &claim_id_of(&lease),
            "aaa1111",
            "rebase onto main conflicts in src/task.rs",
            &json!([{"name": "git rebase", "exit_code": 1}]),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{blocked}");
    assert_eq!(blocked["status"], "blocked", "{blocked}");

    let (status, card) = get_task(&state, "merge:t-a-jam").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        transitions(&card),
        ["cancelled", "dropped"],
        "a blocked merge offers the release and nothing that restarts it: {card}"
    );

    let (status, refused) = post_status(&state, "merge:t-a-jam", "ready").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "pressing ready on a blocked merge: {refused}"
    );
    assert_eq!(refused["code"], "invalid", "{refused}");
    let (status, held) = get_task(&state, "merge:t-a-jam").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(held, card, "the refusal writes nothing: {held}");

    let (status, still_waiting) = claim_kind(&state, "sol", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{still_waiting}");
    assert_eq!(
        still_waiting["status"], "no-work",
        "and the refused press did not let the train move: {still_waiting}"
    );

    // The queue says why it stopped, in the same payload that drew it.
    let plane = control(&state).await;
    assert_eq!(
        ids_of(&plane["pending_merges"]),
        ["merge:t-a-jam", "merge:t-b-behind"],
        "{plane}"
    );
    let merges = plane["pending_merges"]
        .as_array()
        .expect("pending_merges array");
    assert_summary_shape(&merges[0]);
    assert_eq!(
        merges[0]["verification"], "rebase onto main conflicts in src/task.rs",
        "the blocked head carries the reason a screen shows: {plane}"
    );
    assert_eq!(
        merges[1]["verification"],
        Value::Null,
        "a merge that is still running has no reason to show: {plane}"
    );

    // Calling it off is the release, and what follows is a new attempt.
    let (status, cancelled) = post_status(&state, "merge:t-a-jam", "cancelled").await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    let (status, reissued) = post_merge(&state, "t-a-jam").await;
    assert_eq!(status, StatusCode::CREATED, "{reissued}");
    assert_eq!(
        reissued["id"], "merge:t-a-jam~2",
        "the reissue is a new row, not the blocked one reopened: {reissued}"
    );
    assert_eq!(
        reissued["status"], "ready",
        "and it starts clean: {reissued}"
    );
    assert_eq!(reissued["verification"], Value::Null, "{reissued}");
    let (status, failed) = get_task(&state, "merge:t-a-jam").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        failed["verification"], "rebase onto main conflicts in src/task.rs",
        "while the attempt that failed keeps saying why: {failed}"
    );
}

/// The status route cannot say how a merge ended. `done` and `blocked` are the
/// worker's answer, and `POST /worker/report` is the only way to give one.
///
/// This is the outcome side of the same rule the `ready` refusal covers, and it
/// is the side with teeth: pressing `blocked` files a jam that stops the
/// product's train with no reason and no checks on the row, and pressing `done`
/// walks around the check gate and the target landing together, leaving
/// approved work that never merged and that neither reconciliation window on
/// `/api/control` shows — `pending_merges` stops at `done`, and `mergeable`
/// still sees the merge row holding the target.
#[tokio::test]
async fn the_status_route_cannot_write_the_outcome_of_a_merge() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    approved_task(&state, "t-a-merge", PRODUCT, "aaa1111").await;

    // Issued and waiting. The card offers the claim and the two releases, and
    // no outcome at all.
    let (status, issued) = get_task(&state, "merge:t-a-merge").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        transitions(&issued),
        ["wip", "cancelled", "dropped"],
        "an issued merge offers no outcome: {issued}"
    );
    for to in ["done", "blocked"] {
        let (status, refused) = post_status(&state, "merge:t-a-merge", to).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "pressing {to} on an issued merge: {refused}"
        );
        assert_eq!(refused["code"], "invalid", "{refused}");
    }
    let (status, held) = get_task(&state, "merge:t-a-merge").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(held, issued, "the refusals write nothing: {held}");

    // Running. `done` here is the press that would fabricate a landing.
    let (status, lease) = claim_kind(&state, "luna", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    assert_eq!(lease["id"], "merge:t-a-merge", "{lease}");
    let (status, running) = get_task(&state, "merge:t-a-merge").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        transitions(&running),
        ["ready", "cancelled", "dropped"],
        "a running merge offers no outcome either: {running}"
    );
    for to in ["done", "blocked"] {
        let (status, refused) = post_status(&state, "merge:t-a-merge", to).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "pressing {to} on a running merge: {refused}"
        );
        assert_eq!(refused["code"], "invalid", "{refused}");
    }
    assert_merge_did_not_land(&state, "merge:t-a-merge", "t-a-merge").await;
    assert_merge_is_in_flight(&state, "t-a-merge").await;

    // The worker's own report still lands it, gate and target together. It is
    // reported against the lease luna is holding, which is the whole point:
    // only the claim that ran the merge can say how it ended.
    let (status, landed) = post_report(
        &state,
        &report_with_checks(&claim_id_of(&lease), "aaa1111", &green_checks()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{landed}");
    assert_eq!(landed["status"], "done", "{landed}");
    let (status, target) = get_task(&state, "t-a-merge").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        target["status"], "merged",
        "the report is what moves the target: {target}"
    );

    // And a landed merge is not pressed back into a jam.
    let (status, landed) = get_task(&state, "merge:t-a-merge").await;
    assert_eq!(status, StatusCode::OK);
    let (status, refused) = post_status(&state, "merge:t-a-merge", "blocked").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "pressing blocked on a landed merge: {refused}"
    );
    let (status, still_landed) = get_task(&state, "merge:t-a-merge").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(still_landed, landed, "{still_landed}");
    let (status, target) = get_task(&state, "t-a-merge").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(target["status"], "merged", "{target}");
}

/// `outcome` is optional on the wire, and it means one thing on every worker
/// surface: left out it is `done`, and a name that is neither `done` nor
/// `blocked` is refused rather than read as a success.
///
/// Both surfaces read it through the same helper, so this is the contract they
/// share. A fallback on an unrecognised name is the failure that matters: it
/// would file a report nobody understood as a finished one, and on a merge that
/// is a landing.
#[tokio::test]
async fn an_omitted_report_outcome_is_done_and_an_unknown_one_is_refused() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    ready_task(&state, "t-typo", 0).await;
    let claim_id = claim_next(&state, "t-typo").await;

    let (status, refused) = post_report(
        &state,
        &json!({
            "claim_id": claim_id,
            "commit_sha": "abc1234",
            "verification": "cargo test",
            "outcome": "Blocked",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an outcome nobody defined is refused: {refused}"
    );
    assert_eq!(refused["code"], "invalid", "{refused}");
    let (status, held) = get_task(&state, "t-typo").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        held["status"], "wip",
        "and the refusal writes nothing: {held}"
    );

    // Left out entirely, it is the report every worker written before outcomes
    // existed sends.
    let (status, done) = post_report(&state, &report_body(&claim_id, "abc1234")).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        done["status"], "done",
        "an omitted outcome is `done`: {done}"
    );
}

/// The outcome refusal is a merge rule. Ordinary work is still moved by hand
/// over the same route, `blocked` included, and comes back from it.
#[tokio::test]
async fn the_status_route_still_moves_ordinary_work_by_hand() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    ready_task(&state, "t-hand", 0).await;
    set_status(&state, "t-hand", "wip").await;

    let (status, running) = get_task(&state, "t-hand").await;
    assert_eq!(status, StatusCode::OK);
    for to in ["done", "blocked"] {
        assert!(
            transitions(&running).contains(&to.to_owned()),
            "ordinary work still offers {to}: {running}"
        );
    }

    let stopped = set_status(&state, "t-hand", "blocked").await;
    assert_eq!(stopped["status"], "blocked", "{stopped}");
    let restarted = set_status(&state, "t-hand", "ready").await;
    assert_eq!(
        restarted["status"], "ready",
        "blocked ordinary work is restarted by hand: {restarted}"
    );
    set_status(&state, "t-hand", "wip").await;
    let finished = set_status(&state, "t-hand", "done").await;
    assert_eq!(finished["status"], "done", "{finished}");
}

/// A worker loop takes the kinds of work it handles. Left out, it still takes
/// anything, so a loop written before roles existed keeps working.
#[tokio::test]
async fn a_claim_takes_only_the_kinds_the_worker_asks_for() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    create_task(
        &state,
        &json!({"id": "t-reviewed", "title": "reviewed", "product_id": PRODUCT}),
    )
    .await;
    set_status(&state, "t-reviewed", "ready").await;
    work_to_done(&state, "t-reviewed", "abc1234").await;
    ready_task(&state, "t-plain", 0).await;
    issued_review(&state, "t-reviewed").await;

    let (status, none) = claim_kind(&state, "luna", &json!(["instant:merge"])).await;
    assert_eq!(status, StatusCode::OK, "{none}");
    assert_eq!(
        none["status"], "no-work",
        "no work of that kind is an answer, not somebody else's task: {none}"
    );

    let (status, reviewing) = claim_kind(&state, "sol", &json!(["review"])).await;
    assert_eq!(status, StatusCode::OK, "{reviewing}");
    assert_eq!(reviewing["id"], "review:t-reviewed");
    assert_eq!(reviewing["kind"], "review");

    let (status, working) = claim_kind(&state, "opus", &json!(["normal"])).await;
    assert_eq!(status, StatusCode::OK, "{working}");
    assert_eq!(working["id"], "t-plain");

    let (status, refused) = claim_kind(&state, "opus", &json!(["not-a-kind"])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["code"], "invalid", "{refused}");

    // No kinds at all is the old contract, and still takes whatever is next.
    ready_task(&state, "t-any", 0).await;
    let (status, anything) =
        send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "{anything}");
    assert_eq!(anything["id"], "t-any");
}

/// A task that waits for another is promoted by that task's landing, never by a
/// hand: `ready` is refused with `dependency_pending` while the dependency is
/// open, and the card says what the dependency is doing.
#[tokio::test]
async fn a_dependant_is_promoted_by_the_landing_and_refuses_a_pressed_ready() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;
    create_task(
        &state,
        &json!({"id": "t-first", "title": "goes first", "product_id": PRODUCT}),
    )
    .await;
    let second = create_task(
        &state,
        &json!({
            "id": "t-second",
            "title": "waits for the first",
            "product_id": PRODUCT,
            "depends_on": "t-first",
        }),
    )
    .await;
    assert_eq!(second["depends_on"], "t-first", "{second}");
    assert_eq!(second["dependency_status"], "draft", "{second}");

    let (status, refused) = post_status(&state, "t-second", "ready").await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "dependency_pending");
    assert!(
        refused["error"].as_str().unwrap().contains("t-first")
            && refused["error"].as_str().unwrap().contains("draft"),
        "the refusal names the dependency and its status: {refused}"
    );

    for (id, depends_on) in [("t-self", "t-self"), ("t-ghost", "nope")] {
        let (status, refused) = send(
            &state,
            human(
                "POST",
                "/api/tasks",
                &json!({"id": id, "title": "x", "product_id": PRODUCT, "depends_on": depends_on}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{id}: {refused}");
    }

    // t-first already exists, so it is driven by hand rather than through
    // drive_to_merged, which would file it again.
    set_status(&state, "t-first", "ready").await;
    work_to_done(&state, "t-first", "aaa1111").await;
    approve_task(&state, "t-first", "aaa1111").await;
    let merge = issued_merge(&state, "t-first").await;
    land_merge(&state, merge["id"].as_str().unwrap(), "aaa1111").await;
    let (_, promoted) = get_task(&state, "t-second").await;
    assert_eq!(
        promoted["status"], "ready",
        "the landing promoted it: {promoted}"
    );
    assert!(promoted.get("dependency_status").is_none(), "{promoted}");

    let (status, listing) = send(&state, read("/api/tasks")).await;
    assert_eq!(status, StatusCode::OK);
    let row = listing
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "t-second")
        .expect("summary");
    assert_eq!(
        row["depends_on"], "t-first",
        "the list carries the dependency: {row}"
    );

    // A cleared dependency is a PATCH with an explicit null.
    create_task(
        &state,
        &json!({"id": "t-third", "title": "x", "product_id": PRODUCT, "depends_on": "t-second"}),
    )
    .await;
    let (status, cleared) = send(
        &state,
        human("PATCH", "/api/tasks/t-third", &json!({"depends_on": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["depends_on"], Value::Null);
    set_status(&state, "t-third", "ready").await;
}

/// A worker that is about to go hands its claim back, and the task is taken
/// again by the next claim instead of waiting out the lease.
#[tokio::test]
async fn a_worker_hands_a_live_claim_back_and_the_next_claim_takes_the_task() {
    let (_dir, state, clock) = clocked_state(60);
    put_product(&state, PRODUCT, true).await;
    ready_task(&state, "t-1", 0).await;

    let (status, lease) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    let claim_id = claim_id_of(&lease);

    let (status, back) = send(
        &state,
        worker(
            "/worker/claim/release",
            &json!({"claim_id": claim_id, "reason": "self-update"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{back}");
    assert_eq!(back["status"], "ready");
    assert_eq!(back["claim_id"], Value::Null);
    assert_eq!(back["claimed_by"], Value::Null);
    assert_eq!(back["verification"], "claim released by grok: self-update");

    let (status, again) = send(
        &state,
        worker(
            "/worker/claim/release",
            &json!({"claim_id": claim_id, "reason": "shutdown"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{again}");
    assert_eq!(again["code"], "claim_not_live");

    let (status, retaken) = send(&state, worker("/worker/claim", &json!({"worker": "sol"}))).await;
    assert_eq!(status, StatusCode::OK, "{retaken}");
    assert_eq!(retaken["id"], "t-1");
    assert_eq!(retaken["claimed_by"], "sol");
    let retaken_claim = claim_id_of(&retaken);
    assert_ne!(retaken_claim, claim_id);

    // Past the lease the claim is no longer live, and the row is not touched.
    clock.advance_secs(120);
    let (status, late) = send(
        &state,
        worker(
            "/worker/claim/release",
            &json!({"claim_id": retaken_claim, "reason": "shutdown"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{late}");
    assert_eq!(late["code"], "claim_not_live");
    let (_, card) = get_task(&state, "t-1").await;
    assert_eq!(card["status"], "wip");
}
