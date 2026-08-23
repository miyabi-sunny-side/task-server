use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Error;
use crate::product::{self, Product};
use crate::state::AppState;
use crate::task::{
    self, Check, NewTask, Releasable, ReviewOutcome, ReviewVerdict, Task, TaskKind, TaskPatch,
    TaskStatus,
};

/// A row in the task list: enough to render a card in a list, no body.
#[derive(Debug, Serialize)]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    pub kind: TaskKind,
    pub product_id: Option<String>,
    pub priority: i64,
    pub updated_at: String,
}

impl From<Task> for TaskSummary {
    fn from(task: Task) -> Self {
        Self {
            id: task.id,
            title: task.title,
            status: task.status,
            kind: task.kind,
            product_id: task.product_id,
            priority: task.priority,
            updated_at: task.updated_at,
        }
    }
}

/// The full task plus the transitions a human may actually press and, for work
/// a review has answered, what that review said.
#[derive(Debug, Serialize)]
pub struct TaskCard {
    #[serde(flatten)]
    pub task: Task,
    pub available_transitions: Vec<TaskStatus>,
    /// The latest finished review of this task, read from that review's row.
    /// Absent while nothing has reviewed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_review: Option<ReviewOutcome>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimBody {
    pub worker: String,
    /// Which kinds of work this loop handles. Empty or absent takes anything,
    /// so a worker written before roles existed keeps working.
    #[serde(default)]
    pub kinds: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReportBody {
    pub claim_id: String,
    pub commit_sha: String,
    pub verification: String,
    #[serde(default)]
    pub checks: Vec<Check>,
}

/// A review's completion. Deliberately not [`ReportBody`]: a verdict is not a
/// commit, and `request_changes` is a finished review rather than a failed one.
#[derive(Debug, Deserialize)]
pub struct ReviewReportBody {
    pub claim_id: String,
    /// The commit the reviewer read. Must be the one the review was issued for.
    pub subject_commit_sha: String,
    /// `approve` or `request_changes`.
    pub verdict: String,
    /// What the reviewer found, kept whichever way the verdict went.
    pub findings: String,
}

#[derive(Debug, Deserialize)]
pub struct MergeBody {
    pub task_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseBody {
    pub product_id: String,
    pub tag: String,
}

/// What the admin screen needs to decide whether "merge" and "release" are
/// live buttons.
#[derive(Debug, Serialize)]
pub struct ControlPlane {
    pub mergeable: Vec<TaskSummary>,
    pub pending_merges: Vec<TaskSummary>,
    pub releasable: Vec<Releasable>,
}

#[derive(Debug, Serialize)]
pub struct ReleaseResult {
    pub product_id: String,
    pub tag: String,
    pub released: Vec<TaskSummary>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskBody {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub product_id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct StatusBody {
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct ProductBody {
    pub repository: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "releases_by_default")]
    pub releases: bool,
}

fn releases_by_default() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn require_worker(headers: &HeaderMap, state: &AppState) -> Result<(), Error> {
    let provided = header_value(headers, "x-worker-capability");
    if state.worker_capability.is_empty() {
        return Err(Error::Unauthorized);
    }
    if provided == Some(state.worker_capability.as_str()) {
        Ok(())
    } else {
        Err(Error::Unauthorized)
    }
}

fn ingress_identity<'a>(headers: &'a HeaderMap, state: &'a AppState) -> Option<&'a str> {
    header_value(headers, "x-auth-user")
        .or_else(|| header_value(headers, "tailscale-user-login"))
        .or(state.dev_identity.as_deref())
}

/// The name the ingress put on the request. Which names may reach this server
/// at all is the reverse proxy's call — it is the one that can tell a LAN
/// client from anything else — so all that is asked here is that a name came.
fn require_identity(headers: &HeaderMap, state: &AppState) -> Result<String, Error> {
    ingress_identity(headers, state)
        .map(ToOwned::to_owned)
        .ok_or(Error::Unauthorized)
}

/// Identity, then the CSRF token. `Origin` is not read: the token is what a
/// cross-site page cannot produce, and refusing requests that carry no
/// `Origin` at all would only turn away `curl` while stopping nobody.
fn require_human_mutation(headers: &HeaderMap, state: &AppState) -> Result<(), Error> {
    require_identity(headers, state)?;
    let token = header_value(headers, "x-csrf-token");
    if state.csrf_token.is_empty() {
        return Err(Error::Unauthorized);
    }
    if token == Some(state.csrf_token.as_str()) {
        Ok(())
    } else {
        Err(Error::Unauthorized)
    }
}

fn card(db: &Db, task: Task) -> Result<TaskCard, Error> {
    let available_transitions = task::available_transitions(&task);
    // Ordinary work is the only thing a review answers for, so nothing else
    // pays for the lookup.
    let latest_review = if task.kind == TaskKind::Normal {
        task::latest_review(db, &task.id)?
    } else {
        None
    };
    Ok(TaskCard {
        task,
        available_transitions,
        latest_review,
    })
}

fn summaries(tasks: Vec<Task>) -> Vec<TaskSummary> {
    tasks.into_iter().map(TaskSummary::from).collect()
}

pub async fn healthz() -> &'static str {
    "ok\n"
}

pub async fn api_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

pub async fn api_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Error> {
    let user = require_identity(&headers, &state)?;
    Ok(Json(serde_json::json!({
        "user": user,
        "csrf_token": state.csrf_token,
    })))
}

pub async fn api_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<TaskSummary>>, Error> {
    require_identity(&headers, &state)?;
    let tasks = match query.status {
        Some(raw) => task::list_by_status(&state.db, TaskStatus::parse(&raw)?)?,
        None => task::list_active(&state.db)?,
    };
    Ok(Json(summaries(tasks)))
}

/// Register a task.
///
/// `kind` is still read rather than ignored: a request that asks for
/// `instant:merge` is answered with the domain's refusal naming the control
/// plane, which is what a caller needs to hear, instead of quietly filing
/// something other than what was asked for.
pub async fn api_create_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateTaskBody>,
) -> Result<Response, Error> {
    require_human_mutation(&headers, &state)?;
    let kind = match body.kind.as_deref() {
        Some(raw) => TaskKind::parse(raw)?,
        None => TaskKind::Normal,
    };
    let created = task::create(
        &state.db,
        &NewTask {
            id: body.id,
            title: body.title,
            body: body.body,
            product_id: body.product_id,
            kind,
            priority: body.priority.unwrap_or(0),
        },
        state.clock.now(),
    )?;
    Ok((StatusCode::CREATED, Json(card(&state.db, created)?)).into_response())
}

pub async fn api_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TaskCard>, Error> {
    require_identity(&headers, &state)?;
    let task = task::get(&state.db, &id)?;
    Ok(Json(card(&state.db, task)?))
}

pub async fn api_patch_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(patch): Json<TaskPatch>,
) -> Result<Json<TaskCard>, Error> {
    require_human_mutation(&headers, &state)?;
    let updated = task::update(&state.db, &id, &patch, state.clock.now())?;
    Ok(Json(card(&state.db, updated)?))
}

pub async fn api_set_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> Result<Json<TaskCard>, Error> {
    require_human_mutation(&headers, &state)?;
    let to = TaskStatus::parse(&body.status)?;
    // Landing and shipping are earned, not pressed. The rule itself lives in
    // the domain, so the MCP tool refuses exactly what this route refuses.
    let moved = task::set_status_by_operator(&state.db, &id, to, state.clock.now())?;
    Ok(Json(card(&state.db, moved)?))
}

pub async fn api_control(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ControlPlane>, Error> {
    require_identity(&headers, &state)?;
    Ok(Json(ControlPlane {
        mergeable: summaries(task::mergeable(&state.db)?),
        pending_merges: summaries(task::pending_merges(&state.db)?),
        releasable: task::releasable(&state.db)?,
    }))
}

pub async fn api_issue_merge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MergeBody>,
) -> Result<Response, Error> {
    require_human_mutation(&headers, &state)?;
    let issued = task::issue_merge(&state.db, &body.task_id, state.clock.now())?;
    Ok((StatusCode::CREATED, Json(card(&state.db, issued)?)).into_response())
}

pub async fn api_issue_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MergeBody>,
) -> Result<Response, Error> {
    require_human_mutation(&headers, &state)?;
    let issued = task::issue_review(&state.db, &body.task_id, state.clock.now())?;
    Ok((StatusCode::CREATED, Json(card(&state.db, issued)?)).into_response())
}

pub async fn api_release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ReleaseBody>,
) -> Result<Json<ReleaseResult>, Error> {
    require_human_mutation(&headers, &state)?;
    let released =
        task::release_product(&state.db, &body.product_id, &body.tag, state.clock.now())?;
    Ok(Json(ReleaseResult {
        product_id: body.product_id,
        tag: body.tag,
        released: summaries(released),
    }))
}

pub async fn api_products(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Product>>, Error> {
    require_identity(&headers, &state)?;
    Ok(Json(product::list(&state.db)?))
}

pub async fn api_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Product>, Error> {
    require_identity(&headers, &state)?;
    Ok(Json(product::get(&state.db, &id)?))
}

pub async fn api_put_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ProductBody>,
) -> Result<Json<Product>, Error> {
    require_human_mutation(&headers, &state)?;
    let stored = product::upsert(
        &state.db,
        &Product {
            id,
            repository: body.repository,
            description: body.description,
            releases: body.releases,
            // The mark belongs to the walk of the project tree, not to a caller.
            archived: false,
        },
        state.clock.now(),
    )?;
    Ok(Json(stored))
}

pub async fn worker_claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ClaimBody>,
) -> Result<Response, Error> {
    require_worker(&headers, &state)?;
    let kinds = body
        .kinds
        .iter()
        .map(|raw| TaskKind::parse(raw))
        .collect::<Result<Vec<TaskKind>, Error>>()?;
    let leased = task::claim(
        &state.db,
        &body.worker,
        &kinds,
        state.clock.now(),
        state.claim_ttl_secs,
    )?;
    match leased {
        Some(task) => Ok(Json(card(&state.db, task)?).into_response()),
        None => Ok((
            StatusCode::OK,
            Json(serde_json::json!({ "status": "no-work" })),
        )
            .into_response()),
    }
}

pub async fn worker_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ReportBody>,
) -> Result<Json<TaskCard>, Error> {
    require_worker(&headers, &state)?;
    let reported = task::report(
        &state.db,
        &body.claim_id,
        &body.commit_sha,
        &body.verification,
        &body.checks,
        state.clock.now(),
    )?;
    Ok(Json(card(&state.db, reported)?))
}

/// The review's own completion route.
///
/// It is not `/worker/report` because the two contracts differ: a review answers
/// with a verdict rather than a commit, carries no checks to gate on, and a
/// `request_changes` is a success — the reviewer did their job and the answer is
/// "not yet". The capability is the same one every worker route asks for.
pub async fn worker_review_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ReviewReportBody>,
) -> Result<Json<TaskCard>, Error> {
    require_worker(&headers, &state)?;
    let verdict = ReviewVerdict::parse(&body.verdict)?;
    let reported = task::review_report(
        &state.db,
        &body.claim_id,
        &body.subject_commit_sha,
        verdict,
        &body.findings,
        state.clock.now(),
    )?;
    Ok(Json(card(&state.db, reported)?))
}

/// A path no API route claims. It is our refusal, so it answers in the shape
/// every refusal of ours uses — the same status and the same `not_found` slug as
/// [`Error::NotFound`] — instead of falling through to the client's index.html.
pub async fn api_not_found() -> Response {
    Error::NotFound.into_response()
}
