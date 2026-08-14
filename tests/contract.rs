//! Contract tests for the shipped task-server API.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tempfile::TempDir;
use time::macros::datetime;
use tower::ServiceExt;

use task_server::frontmatter::{get_str, set_str};
use task_server::outbox::list_pending;
use task_server::status::{Status, TransitionContext, can_transition};
use task_server::{
    ActionTable, AppState, FailingNotifier, ReportRequest, SharedClock, apply_human_action, claim,
    join_document, report, self_service_awaiting_user, split_document,
};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn init_repo() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().to_path_buf();
    git(&root, &["init"]);
    git(&root, &["config", "user.name", "test"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    fs::create_dir_all(root.join("projects/queue/tasks")).unwrap();
    (dir, root)
}

fn commit_file(root: &Path, rel: &str, contents: &[u8], message: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    git(root, &["add", "--", rel]);
    git(root, &["commit", "-m", message]);
}

fn task_rel(id: &str) -> String {
    format!("projects/queue/tasks/{id}.md")
}

fn ready_fixture(title: &str, space: &str) -> Vec<u8> {
    // CJK, an extra --- fence, and no trailing newline.
    format!(
        "---\n\
         type: Task\n\
         title: {title}\n\
         status: ready\n\
         area: development\n\
         target_space: {space}\n\
         next_action: implement\n\
         ---\n\
         # {title}\n\
         \n\
         本文に CJK と --- を含む。\n\
         ---\n\
         最後の行"
    )
    .into_bytes()
}

fn read_task(root: &Path, id: &str) -> (task_server::Document, Vec<u8>) {
    let bytes = fs::read(root.join(task_rel(id))).unwrap();
    (split_document(&bytes).unwrap(), bytes)
}

fn awaiting_user_fixture(title: &str, space: &str) -> Vec<u8> {
    format!(
        "---\n\
         type: Task\n\
         title: {title}\n\
         status: awaiting_user\n\
         area: development\n\
         target_space: {space}\n\
         commit_sha: abcdef1\n\
         verification: built\n\
         ---\n\
         # {title}\n\
         \n\
         確認待ち本文"
    )
    .into_bytes()
}

#[test]
fn frontmatter_rewrite_preserves_markdown_body_bytes() {
    let fixture = ready_fixture("本文保存", "workers");
    assert!(
        !fixture.ends_with(b"\n"),
        "fixture must omit a trailing newline"
    );
    let original = split_document(&fixture).expect("split fixture");
    let original_body = original.body.clone();
    let body_text = std::str::from_utf8(&original_body).unwrap();
    assert!(
        original_body.windows(3).any(|window| window == b"---"),
        "body must contain an extra --- fence"
    );
    assert!(
        body_text.contains("本文") && body_text.contains("CJK"),
        "body must keep CJK text: {body_text:?}"
    );

    let mut rewritten = original;
    set_str(&mut rewritten.frontmatter, "status", "running");
    let joined = join_document(&rewritten).expect("join");
    let after = split_document(&joined).expect("split rewritten");
    assert_eq!(after.body, original_body);
    assert_eq!(
        get_str(&after.frontmatter, "status").as_deref(),
        Some("running")
    );
}

#[test]
fn claim_then_report_flips_status_in_one_git_commit_and_preserves_body() {
    let (_tmp, root) = init_repo();
    let fixture = ready_fixture("claim-report", "workers");
    let original_body = split_document(&fixture).unwrap().body;
    commit_file(&root, &task_rel("alpha"), &fixture, "add alpha");
    let before_log = git(&root, &["rev-list", "--count", "HEAD"]);

    let state = AppState::for_test(&root);
    let lease = claim(&state, "grok").expect("claim").expect("lease");
    assert_eq!(lease.task_id, "alpha");
    assert_eq!(lease.status, "running");
    assert_eq!(lease.body.as_bytes(), original_body.as_slice());
    assert!(!lease.claim_id.is_empty());

    let after_claim: u32 = git(&root, &["rev-list", "--count", "HEAD"])
        .trim()
        .parse()
        .unwrap();
    assert!(
        after_claim > before_log.trim().parse().unwrap(),
        "claim itself must commit"
    );

    let outcome = report(
        &state,
        &ReportRequest {
            claim_id: lease.claim_id.clone(),
            commit_sha: "deadbeef".into(),
            verification: "cargo test".into(),
        },
    )
    .expect("report");
    assert_eq!(outcome.status, "awaiting_user");
    assert_eq!(outcome.commit_sha, "deadbeef");

    let (doc, _) = read_task(&root, "alpha");
    assert_eq!(
        get_str(&doc.frontmatter, "status").as_deref(),
        Some("awaiting_user")
    );
    assert_eq!(
        get_str(&doc.frontmatter, "commit_sha").as_deref(),
        Some("deadbeef")
    );
    assert_eq!(
        get_str(&doc.frontmatter, "verification").as_deref(),
        Some("cargo test")
    );
    assert_eq!(doc.body, original_body);

    let after_report: u32 = git(&root, &["rev-list", "--count", "HEAD"])
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        after_report,
        after_claim + 1,
        "report accept + status flip must be exactly one commit"
    );
    let last = git(&root, &["log", "-1", "--format=%s"]);
    assert!(
        last.contains("awaiting_user") || last.contains("report"),
        "flip commit message: {last}"
    );
    let show = git(&root, &["show", "--name-only", "--pretty=format:", "HEAD"]);
    assert!(
        show.contains("projects/queue/tasks/alpha.md"),
        "flip commit must include the task file: {show}"
    );
    let patch = git(&root, &["show", "--format=", "HEAD"]);
    assert!(
        patch.contains("awaiting_user")
            && patch.contains("deadbeef")
            && patch.contains("cargo test"),
        "the single report commit must carry status, commit_sha, and verification: {patch}"
    );
}

#[test]
fn expired_lease_can_be_reclaimed_and_stale_claim_id_report_is_rejected() {
    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("beta"),
        &ready_fixture("reclaim", "workers"),
        "add beta",
    );
    let clock = SharedClock::at(datetime!(2026-08-15 00:00:00 UTC));
    let state = AppState::for_test(&root)
        .with_clock(Arc::new(clock.clone()))
        .with_ttl(60);

    let first = claim(&state, "grok").unwrap().expect("first lease");
    clock.advance_secs(61);
    let second = claim(&state, "grok").unwrap().expect("reclaim");
    assert_ne!(
        first.claim_id, second.claim_id,
        "reclaim must mint a new claim_id"
    );

    let delayed = report(
        &state,
        &ReportRequest {
            claim_id: first.claim_id,
            commit_sha: "1111111".into(),
            verification: "late".into(),
        },
    );
    assert!(delayed.is_err(), "stale claim_id must be rejected");

    let ok = report(
        &state,
        &ReportRequest {
            claim_id: second.claim_id,
            commit_sha: "2222222".into(),
            verification: "on time".into(),
        },
    )
    .expect("fresh report");
    assert_eq!(ok.status, "awaiting_user");
}

#[test]
fn unexpired_claim_is_exclusive() {
    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("gamma"),
        &ready_fixture("exclusive", "workers"),
        "add gamma",
    );
    let state = AppState::for_test(&root);
    let first = claim(&state, "grok").unwrap().expect("first");
    let second = claim(&state, "codex").unwrap();
    assert!(
        second.is_none(),
        "second claim must not steal the same task"
    );
    let (doc, _) = read_task(&root, "gamma");
    assert_eq!(
        get_str(&doc.frontmatter, "claim_id").as_deref(),
        Some(first.claim_id.as_str())
    );
}

#[test]
fn claim_returns_at_most_one_ready_task() {
    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("one"),
        &ready_fixture("one", "workers"),
        "add one",
    );
    commit_file(
        &root,
        &task_rel("two"),
        &ready_fixture("two", "workers"),
        "add two",
    );
    let state = AppState::for_test(&root);
    let lease = claim(&state, "grok").unwrap().expect("one lease");
    assert!(
        lease.task_id == "one" || lease.task_id == "two",
        "claimed unexpected id {}",
        lease.task_id
    );
    let other = if lease.task_id == "one" { "two" } else { "one" };
    let (doc, _) = read_task(&root, other);
    assert_eq!(
        get_str(&doc.frontmatter, "status").as_deref(),
        Some("ready")
    );
    let (claimed_doc, _) = read_task(&root, &lease.task_id);
    assert_eq!(
        get_str(&claimed_doc.frontmatter, "status").as_deref(),
        Some("running")
    );
}

#[test]
fn notify_failure_does_not_roll_back_status_commit_and_outbox_keeps_intent() {
    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("delta"),
        &ready_fixture("notify", "workers"),
        "add delta",
    );
    let state = AppState::for_test(&root).with_notifier(Arc::new(FailingNotifier));
    let lease = claim(&state, "grok").unwrap().expect("lease");
    report(
        &state,
        &ReportRequest {
            claim_id: lease.claim_id,
            commit_sha: "cafebabe".into(),
            verification: "done".into(),
        },
    )
    .expect("report must succeed even if notify fails");

    let (doc, _) = read_task(&root, "delta");
    assert_eq!(
        get_str(&doc.frontmatter, "status").as_deref(),
        Some("awaiting_user")
    );
    let pending = list_pending(&state.outbox_dir).expect("outbox");
    assert!(
        pending.iter().any(|intent| {
            intent.task_id == "delta"
                && intent.state == "pending"
                && intent.commit_sha == "cafebabe"
        }),
        "outbox must keep a pending NotificationIntent: {pending:?}"
    );
}

#[tokio::test]
async fn identity_header_alone_does_not_authenticate_worker_or_human_mutation() {
    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("eps"),
        &awaiting_user_fixture("authz", "workers"),
        "add eps",
    );
    let state = AppState::for_test(&root);
    let app = task_server::app(state);

    let worker = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/worker/claim")
                .header("content-type", "application/json")
                .header("x-auth-user", "miyabi")
                .body(Body::from(r#"{"worker":"grok"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        worker.status() == StatusCode::UNAUTHORIZED || worker.status() == StatusCode::FORBIDDEN,
        "identity alone must not authenticate worker claim: {}",
        worker.status()
    );

    let human = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/tasks/eps/actions/done")
                .header("content-type", "application/json")
                .header("x-auth-user", "miyabi")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        human.status() == StatusCode::UNAUTHORIZED || human.status() == StatusCode::FORBIDDEN,
        "identity alone must not authenticate human mutation: {}",
        human.status()
    );
}

#[tokio::test]
async fn worker_capability_cannot_post_human_action() {
    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("zeta"),
        &awaiting_user_fixture("cap", "workers"),
        "add zeta",
    );
    let state = AppState::for_test(&root);
    let app = task_server::app(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/tasks/zeta/actions/done")
                .header("content-type", "application/json")
                .header("x-worker-capability", "test-capability")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status() == StatusCode::UNAUTHORIZED || response.status() == StatusCode::FORBIDDEN,
        "worker capability must not post a human action: {}",
        response.status()
    );
}

#[test]
fn awaiting_user_actions_translate_to_canonical_status() {
    let table = ActionTable::default();
    let done = table
        .translate(Status::AwaitingUser, "done", None)
        .expect("done");
    assert_eq!(done.to, Status::Done);

    let push = table
        .translate(Status::AwaitingUser, "push", None)
        .expect("push");
    assert_eq!(push.to, Status::Ready);
    assert_eq!(push.next_action.as_deref(), Some("push"));

    for bump in ["patch", "minor", "major"] {
        let effect = table
            .translate(Status::AwaitingUser, "bump-tag", Some(bump))
            .unwrap_or_else(|_| panic!("bump-tag {bump}"));
        assert_eq!(effect.to, Status::ReleaseRequested);
        assert_eq!(effect.bump.as_deref(), Some(bump));
        assert!(effect.set_release);
    }

    let more = table
        .translate(Status::AwaitingUser, "ask-more", None)
        .expect("ask-more");
    assert_eq!(more.to, Status::Ready);
    assert_eq!(more.next_action.as_deref(), Some("ask-more"));

    assert!(
        table
            .translate(Status::AwaitingUser, "explode", None)
            .is_err()
    );
    assert!(table.translate(Status::Ready, "done", None).is_err());

    let available = table.available_actions(Status::AwaitingUser);
    for name in ["done", "push", "bump-tag", "ask-more"] {
        assert!(
            available.iter().any(|item| item == name),
            "{name} must come from the same table: {available:?}"
        );
    }

    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("act-done"),
        &awaiting_user_fixture("done", "workers"),
        "add act-done",
    );
    commit_file(
        &root,
        &task_rel("act-push"),
        &awaiting_user_fixture("push", "workers"),
        "add act-push",
    );
    commit_file(
        &root,
        &task_rel("act-ask"),
        &awaiting_user_fixture("ask", "workers"),
        "add act-ask",
    );
    commit_file(
        &root,
        &task_rel("act-bump"),
        &awaiting_user_fixture("bump", "workers"),
        "add act-bump",
    );
    let state = AppState::for_test(&root);

    let card = apply_human_action(&state, "act-done", "done", None).expect("apply done");
    assert_eq!(card.status, "done");
    assert_eq!(
        get_str(&read_task(&root, "act-done").0.frontmatter, "status").as_deref(),
        Some("done")
    );

    let card = apply_human_action(&state, "act-push", "push", None).expect("apply push");
    assert_eq!(card.status, "ready");
    let doc = read_task(&root, "act-push").0;
    assert_eq!(
        get_str(&doc.frontmatter, "next_action").as_deref(),
        Some("push")
    );

    let card = apply_human_action(&state, "act-ask", "ask-more", None).expect("apply ask-more");
    assert_eq!(card.status, "ready");
    let doc = read_task(&root, "act-ask").0;
    assert_eq!(
        get_str(&doc.frontmatter, "next_action").as_deref(),
        Some("ask-more")
    );

    let card = apply_human_action(&state, "act-bump", "bump-tag", Some("minor")).expect("bump");
    assert_eq!(card.status, "release_requested");
    let doc = read_task(&root, "act-bump").0;
    assert_eq!(get_str(&doc.frontmatter, "bump").as_deref(), Some("minor"));
    assert_eq!(
        get_str(&doc.frontmatter, "release_sha").as_deref(),
        Some("abcdef1")
    );
    assert_eq!(
        get_str(&doc.frontmatter, "release_repo").as_deref(),
        Some("workers")
    );

    assert!(apply_human_action(&state, "act-done", "explode", None).is_err());
}

#[test]
fn self_service_ready_to_awaiting_user_is_allowed_when_target_space_is_tasks() {
    let tasks_ctx = TransitionContext {
        target_space: Some("tasks".into()),
        product_id: None,
    };
    assert!(can_transition(
        Status::Ready,
        Status::AwaitingUser,
        &tasks_ctx
    ));
    let workers_ctx = TransitionContext {
        target_space: Some("workers".into()),
        product_id: None,
    };
    assert!(!can_transition(
        Status::Ready,
        Status::AwaitingUser,
        &workers_ctx
    ));

    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("self"),
        &ready_fixture("self-service", "tasks"),
        "add self",
    );
    commit_file(
        &root,
        &task_rel("other"),
        &ready_fixture("not-self", "workers"),
        "add other",
    );
    let state = AppState::for_test(&root);
    let ok = self_service_awaiting_user(&state, "self", "abc1234", "self flip").expect("self");
    assert_eq!(ok.status, "awaiting_user");
    let doc = read_task(&root, "self").0;
    assert_eq!(
        get_str(&doc.frontmatter, "status").as_deref(),
        Some("awaiting_user")
    );
    assert_eq!(
        get_str(&doc.frontmatter, "commit_sha").as_deref(),
        Some("abc1234")
    );
    assert!(self_service_awaiting_user(&state, "other", "abc1234", "nope").is_err());
}

#[tokio::test]
async fn worker_claim_http_returns_a_real_task() {
    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("http1"),
        &ready_fixture("via-http", "workers"),
        "add http1",
    );
    let state = AppState::for_test(&root);
    let app = task_server::app(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/worker/claim")
                .header("content-type", "application/json")
                .header("x-worker-capability", "test-capability")
                .body(Body::from(r#"{"worker":"grok"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["task_id"], "http1");
    assert_eq!(value["status"], "running");
    assert!(value["claim_id"].as_str().is_some_and(|id| !id.is_empty()));
    assert!(
        value["title"]
            .as_str()
            .is_some_and(|title| title.contains("via-http"))
    );
    assert!(
        value["body"]
            .as_str()
            .is_some_and(|text| text.contains("CJK"))
    );
}

#[tokio::test]
async fn report_is_idempotent_for_the_same_claim_and_sha() {
    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("idem"),
        &ready_fixture("idem", "workers"),
        "add idem",
    );
    let state = AppState::for_test(&root);
    let lease = claim(&state, "grok").unwrap().expect("lease");
    let req = ReportRequest {
        claim_id: lease.claim_id,
        commit_sha: "eeeeeee".into(),
        verification: "once".into(),
    };
    let first = report(&state, &req).unwrap();
    let count_after_first: u32 = git(&root, &["rev-list", "--count", "HEAD"])
        .trim()
        .parse()
        .unwrap();
    let second = report(&state, &req).unwrap();
    assert_eq!(first.status, second.status);
    let count_after_second: u32 = git(&root, &["rev-list", "--count", "HEAD"])
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        count_after_first, count_after_second,
        "idempotent report must not add another transition commit"
    );
}

#[tokio::test]
async fn human_mutation_requires_origin_and_csrf() {
    let (_tmp, root) = init_repo();
    commit_file(
        &root,
        &task_rel("origin"),
        &awaiting_user_fixture("origin", "workers"),
        "add origin",
    );
    let state = AppState::for_test(&root);
    let app = task_server::app(state);

    let missing_origin = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/tasks/origin/actions/done")
                .header("content-type", "application/json")
                .header("x-auth-user", "miyabi")
                .header("x-csrf-token", "test-csrf")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        missing_origin.status() == StatusCode::UNAUTHORIZED
            || missing_origin.status() == StatusCode::FORBIDDEN,
        "missing Origin must fail: {}",
        missing_origin.status()
    );

    let wrong_origin = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/tasks/origin/actions/done")
                .header("content-type", "application/json")
                .header("x-auth-user", "miyabi")
                .header("x-csrf-token", "test-csrf")
                .header("origin", "https://evil.example")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        wrong_origin.status() == StatusCode::UNAUTHORIZED
            || wrong_origin.status() == StatusCode::FORBIDDEN,
        "wrong Origin must fail: {}",
        wrong_origin.status()
    );

    let ok = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/tasks/origin/actions/done")
                .header("content-type", "application/json")
                .header("x-auth-user", "miyabi")
                .header("x-csrf-token", "test-csrf")
                .header("origin", "https://task-server.test")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let body = to_bytes(ok.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["status"], "done");
}

#[test]
fn production_startup_is_fail_closed_without_secrets() {
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
        "NTFY_URL" => Some("http://127.0.0.1:9/topic".into()),
        _ => None,
    })
    .expect("production with secrets");
    assert!(ok.dev_identity.is_none());
    assert_eq!(ok.worker_capability, "secret-cap");
}

#[test]
fn action_table_is_loaded_from_json_not_hardcoded_match() {
    assert_eq!(
        ActionTable::from_json(ActionTable::shipped_json().as_bytes())
            .unwrap()
            .available_actions(Status::AwaitingUser),
        ActionTable::default().available_actions(Status::AwaitingUser)
    );

    let custom = br#"[
      {"action":"ship-it","from":"awaiting_user","to":"done"}
    ]"#;
    let table = ActionTable::from_json(custom).expect("custom table");
    let effect = table
        .translate(Status::AwaitingUser, "ship-it", None)
        .expect("custom action");
    assert_eq!(effect.to, Status::Done);
    assert!(table.translate(Status::AwaitingUser, "done", None).is_err());
    assert_eq!(
        table.available_actions(Status::AwaitingUser),
        vec!["ship-it".to_string()]
    );
}

#[test]
fn canonical_okf_fixtures_pass_task_server_and_tasks_check() {
    use task_server::validate_task;

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/okf-good");
    assert!(
        fixture.join("projects/x/index.md").is_file(),
        "canonical fixture bytes must be vendored at tests/fixtures/okf-good"
    );
    let mut checked = 0;
    for entry in fs::read_dir(fixture.join("projects/x/tasks")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let doc = split_document(&bytes).unwrap_or_else(|err| panic!("{path:?}: {err}"));
        validate_task(&doc).unwrap_or_else(|err| panic!("{path:?}: {err}"));
        checked += 1;
    }
    assert!(checked > 0, "expected Task files in the shared fixture");

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let check = ["../tasks/bin/check", "../../household/tasks/bin/check"]
        .into_iter()
        .map(|rel| manifest.join(rel))
        .find(|path| path.is_file())
        .expect("tasks bin/check as a layout-relative neighbor (no absolute household path)");
    let output = Command::new(&check)
        .arg(&fixture)
        .output()
        .expect("run tasks bin/check");
    assert!(
        output.status.success(),
        "tasks bin/check must accept the same fixture bytes: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn http_notifier_retries_after_failure_and_marks_delivered() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(false).unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_for_thread = hits.clone();
    thread::spawn(move || {
        while hits_for_thread.load(Ordering::SeqCst) < 2 {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);
            let n = hits_for_thread.fetch_add(1, Ordering::SeqCst);
            let status = if n == 0 {
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            };
            let _ = stream.write_all(status.as_bytes());
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    });
    thread::sleep(std::time::Duration::from_millis(20));

    let dir = tempfile::TempDir::new().unwrap();
    let outbox = dir.path().join("outbox");
    let intent = task_server::NotificationIntent {
        task_id: "retry".into(),
        kind: "awaiting_user".into(),
        commit_sha: "abc1234".into(),
        claim_id: "claim-1".into(),
        created_at: "2026-08-15T00:00:00Z".into(),
        state: "pending".into(),
    };
    task_server::outbox::enqueue(&outbox, &intent).unwrap();
    let notifier = task_server::HttpNotifier {
        url: format!("http://{addr}/topic"),
    };
    let first = task_server::flush_pending(&outbox, &notifier).unwrap();
    assert_eq!(first, 0, "first attempt must stay pending");
    let pending = list_pending(&outbox).unwrap();
    assert_eq!(pending.len(), 1);
    let second = task_server::flush_pending(&outbox, &notifier).unwrap();
    assert_eq!(second, 1, "retry must deliver once");
    assert!(list_pending(&outbox).unwrap().is_empty());
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
