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
    let claim_id = lease["claim_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .expect("claim_id")
        .to_owned();

    let (status, reported) = send(
        &state,
        worker("/worker/report", &report_body(&claim_id, "abc1234")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "report: {reported}");
    assert_eq!(reported["status"], "done");
    assert_eq!(reported["commit_sha"], "abc1234");
    assert_eq!(reported["verification"], "cargo test");

    let merged = set_status(&state, "t-cutover", "merged").await;
    assert!(
        transitions(&merged).contains(&"released".to_owned()),
        "a merged task of a releasing product can be released: {merged}"
    );

    let released = set_status(&state, "t-cutover", "released").await;
    assert!(
        transitions(&released).is_empty(),
        "released is terminal: {released}"
    );

    let state = reopen(&dir, state);
    let (status, final_card) = send(&state, read("/api/tasks/t-cutover")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(final_card["status"], "released");
    assert_eq!(final_card["branch"], "task/t-cutover");
}

#[tokio::test]
async fn claim_prefers_instant_merge_and_listing_hides_released() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    for (id, kind, priority) in [
        ("t-normal", "normal", 100),
        ("t-instant", "instant:merge", 0),
    ] {
        create_task(
            &state,
            &json!({
                "id": id,
                "title": format!("task {id}"),
                "product_id": PRODUCT,
                "kind": kind,
                "priority": priority,
            }),
        )
        .await;
        set_status(&state, id, "ready").await;
    }

    let (status, lease) = send(&state, worker("/worker/claim", &json!({"worker": "grok"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        lease["id"], "t-instant",
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

    let (status, _) = send(
        &state,
        worker("/worker/report", &report_body(&claim_id, "abc1234")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    set_status(&state, "t-instant", "merged").await;
    set_status(&state, "t-instant", "released").await;

    let (status, listing) = send(&state, read("/api/tasks")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        ids_of(&listing),
        ["t-normal"],
        "the default listing hides released"
    );
    let summary = &listing.as_array().unwrap()[0];
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

    let (status, released) = send(&state, read("/api/tasks?status=released")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids_of(&released), ["t-instant"]);

    let (status, _) = send(&state, read("/api/tasks?status=not-a-status")).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unknown status must be 400"
    );
}

#[tokio::test]
async fn mutation_requires_identity_origin_and_csrf_while_worker_requires_capability() {
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
    let (status, _) = send(&state, no_identity).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "identity is required");

    let wrong_origin = request("POST", "/api/tasks")
        .header("x-auth-user", USER)
        .header("origin", "https://evil.example")
        .header("x-csrf-token", CSRF)
        .body(Body::from(denied.to_string()))
        .unwrap();
    let (status, _) = send(&state, wrong_origin).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a foreign Origin is refused");

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

/// Authorization has to check the values, not merely the presence of headers,
/// and a refused mutation must leave nothing behind.
#[tokio::test]
async fn mutations_refuse_wrong_header_values_without_writing() {
    let (_dir, state) = file_backed_state();
    put_product(&state, PRODUCT, true).await;

    let denied = |id: &str| {
        json!({
            "id": id,
            "title": "must not exist",
            "product_id": PRODUCT,
        })
    };

    // An identity outside the allowlist is not an identity.
    let intruder = request("POST", "/api/tasks")
        .header("x-auth-user", "intruder")
        .header("origin", ORIGIN)
        .header("x-csrf-token", CSRF)
        .body(Body::from(denied("t-intruder").to_string()))
        .unwrap();
    let (status, _) = send(&state, intruder).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an identity outside the allowlist is refused"
    );

    // The ingress header the reverse proxy sets is checked the same way.
    let intruder_ingress = request("POST", "/api/tasks")
        .header("tailscale-user-login", "intruder")
        .header("origin", ORIGIN)
        .header("x-csrf-token", CSRF)
        .body(Body::from(denied("t-ingress").to_string()))
        .unwrap();
    let (status, _) = send(&state, intruder_ingress).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an unlisted ingress identity is refused"
    );

    // A missing Origin is refused, so a bare cross-site form post cannot write.
    let no_origin = request("POST", "/api/tasks")
        .header("x-auth-user", USER)
        .header("x-csrf-token", CSRF)
        .body(Body::from(denied("t-no-origin").to_string()))
        .unwrap();
    let (status, _) = send(&state, no_origin).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a mutation without an Origin is refused"
    );

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

    for id in ["t-intruder", "t-ingress", "t-no-origin", "t-wrong-csrf"] {
        let (status, leaked) = send(&state, read(&format!("/api/tasks/{id}"))).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a refused mutation must not have written {id}: {leaked}"
        );
    }
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
        "APP_AUTH_ALLOWLIST" => Some("miyabi".into()),
        "APP_CSRF_TOKEN" => Some("secret-csrf".into()),
        "APP_ALLOWED_ORIGINS" => Some("https://task-server.test".into()),
        "APP_DB_PATH" => Some(db_path.to_string_lossy().into_owned()),
        _ => None,
    })
    .expect("production with secrets");
    assert!(ok.dev_identity.is_none());
    assert_eq!(ok.worker_capability, "secret-cap");
    assert!(db_path.is_file(), "APP_DB_PATH must be opened at startup");
}
