use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Error;
use crate::product::{self, Product};
use crate::runs::{self, NewRun};
use crate::state::AppState;
use crate::task::{
    self, BlockedBy, Check, NewTask, Releasable, ReleaseLevel, ReportOutcome, ReviewOutcome,
    ReviewVerdict, Stuck, Task, TaskKind, TaskPatch, TaskStatus,
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
    /// The task this one waits for, so a list can say why a draft is waiting.
    pub depends_on: Option<String>,
    /// Who put a `blocked` task there, so a list can tell parked from stuck.
    pub blocked_by: Option<BlockedBy>,
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
            depends_on: task.depends_on,
            blocked_by: task.blocked_by,
        }
    }
}

/// A merge the control plane is carrying: the ordinary summary, plus why it
/// stopped if it did.
///
/// The reason is on the merge task's own `verification`, written there by the
/// worker's blocked report, and this is the one list that has to show it: a
/// stopped merge holds up every merge of its product, so the screen drawing the
/// train cannot say what is happening without it. Carried here rather than
/// added to [`TaskSummary`] because a summary is what every list of tasks
/// carries and this is what *this* list needs — flattened, so the summary stays
/// the single definition of what a row is and the wire shape gains one key
/// rather than a nested object.
#[derive(Debug, Serialize)]
pub struct PendingMerge {
    #[serde(flatten)]
    pub summary: TaskSummary,
    /// Why this merge stopped, as the worker wrote it. `null` while it is
    /// running: only a blocked merge has a reason, and a merge is blocked only
    /// by a report that had one.
    pub verification: Option<String>,
}

impl From<Task> for PendingMerge {
    fn from(task: Task) -> Self {
        Self {
            verification: task.verification.clone(),
            summary: task.into(),
        }
    }
}

/// A release the control plane is carrying: the summary, how far it steps the
/// version, and why it stopped if it did — the same shape as [`PendingMerge`],
/// for the same screen.
#[derive(Debug, Serialize)]
pub struct PendingRelease {
    #[serde(flatten)]
    pub summary: TaskSummary,
    pub release_level: ReleaseLevel,
    pub verification: Option<String>,
}

impl From<Task> for PendingRelease {
    fn from(task: Task) -> Self {
        Self {
            release_level: task.release_level,
            verification: task.verification.clone(),
            summary: task.into(),
        }
    }
}

/// A row of the done screen: what a `normal` task finished, and when.
///
/// Not [`TaskSummary`] plus fields, because a summary is what every list of
/// live work carries and this list is read after the pipeline stopped
/// tracking the task — `updated_at` keeps moving through merge and release,
/// so it is `done_at` this row sorts and shows, not `updated_at`.
#[derive(Debug, Serialize)]
pub struct DoneSummary {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    pub product_id: Option<String>,
    pub release_tag: Option<String>,
    pub verification: Option<String>,
    /// When this task first reached `done`. Present on every row this
    /// endpoint returns — [`task::list_done`] selects only rows that did.
    pub done_at: Option<String>,
}

impl From<Task> for DoneSummary {
    fn from(task: Task) -> Self {
        Self {
            id: task.id,
            title: task.title,
            status: task.status,
            product_id: task.product_id,
            release_tag: task.release_tag,
            verification: task.verification,
            done_at: task.done_at,
        }
    }
}

/// A row of the closed screen: finished or called-off `normal` work, and when it
/// closed. `closed_at` is the sort key (`done_at` for finished work, the moment
/// of cancelling otherwise); `done_at` stays for readers of the done screen.
#[derive(Debug, Serialize)]
pub struct ClosedSummary {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    pub product_id: Option<String>,
    pub release_tag: Option<String>,
    pub verification: Option<String>,
    pub done_at: Option<String>,
    pub closed_at: String,
}

impl From<Task> for ClosedSummary {
    fn from(task: Task) -> Self {
        let closed_at = task::closed_moment(&task);
        Self {
            id: task.id,
            title: task.title,
            status: task.status,
            product_id: task.product_id,
            release_tag: task.release_tag,
            verification: task.verification,
            done_at: task.done_at,
            closed_at,
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
    /// The status of the dependency this task is still waiting for. Absent
    /// when it has none or that task has landed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_status: Option<TaskStatus>,
    /// How many haystack rows name this task (`GET /api/runs?task_id=`).
    pub runs_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct ClaimBody {
    pub worker: String,
    /// Which kinds of work this loop handles. Empty or absent takes anything,
    /// so a worker written before roles existed keeps working.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// A fresh key for one logical claim attempt. Retrying it recovers the same
    /// live lease when the first response was lost.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReportBody {
    pub claim_id: String,
    pub commit_sha: String,
    /// What the worker ran, or — when `outcome` is `blocked` — why it could not
    /// finish. Either way it is the evidence a human reads off the task.
    pub verification: String,
    #[serde(default)]
    pub checks: Vec<Check>,
    /// `done` (the default, and what a worker written before this sent) or
    /// `blocked`: the work could not be finished, and the reason and checks are
    /// to be kept on the task rather than thrown away with a refusal.
    #[serde(default)]
    pub outcome: Option<String>,
    /// The tag a release cut. Required on a `done` report of an
    /// `instant:release` task, and meaningless on every other kind.
    #[serde(default)]
    pub release_tag: Option<String>,
}

/// A worker handing a live claim back before its lease runs out.
#[derive(Debug, Deserialize)]
pub struct ClaimReleaseBody {
    pub claim_id: String,
    /// Why, in a short fixed word (`shutdown`, `self-update`, `gave-up`). Kept
    /// on the task's `verification`.
    pub reason: String,
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
}

/// What the admin screen needs to show what the control plane is carrying.
///
/// Reviews, merges, and releases are issued by the control plane itself, so the
/// three `pending_*` lists are what is in flight rather than what a human is
/// being asked to press. `mergeable`, `unreviewed`, and `releasable` are
/// reconciliation windows: all are empty while the automatic issuing works, and
/// anything in any of them is work that lost its next step.
#[derive(Debug, Serialize)]
pub struct ControlPlane {
    pub mergeable: Vec<TaskSummary>,
    pub pending_merges: Vec<PendingMerge>,
    pub pending_releases: Vec<PendingRelease>,
    pub pending_reviews: Vec<TaskSummary>,
    pub unreviewed: Vec<TaskSummary>,
    pub releasable: Vec<Releasable>,
    /// Work the control plane is holding past its threshold, with a fixed
    /// reason each. Empty while everything moves.
    pub stuck: Vec<Stuck>,
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
    #[serde(default)]
    pub release_level: Option<String>,
    #[serde(default)]
    pub depends_on: Option<String>,
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
    let available_transitions = task::offered_transitions(db, &task)?;
    // Ordinary work is the only thing a review answers for, so nothing else
    // pays for the lookup.
    let latest_review = if task.kind == TaskKind::Normal {
        task::latest_review(db, &task.id)?
    } else {
        None
    };
    let dependency_status = task::dependency_status(db, &task)?;
    let runs_count = runs::count_for_task(db, &task.id)?;
    Ok(TaskCard {
        task,
        available_transitions,
        latest_review,
        dependency_status,
        runs_count,
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

/// Completed `normal` work, most recently finished first (`GET /api/done`).
pub async fn api_done(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<DoneSummary>>, Error> {
    require_identity(&headers, &state)?;
    let tasks = task::list_done(&state.db)?;
    Ok(Json(tasks.into_iter().map(DoneSummary::from).collect()))
}

/// Finished and called-off `normal` work, most recently closed first
/// (`GET /api/closed`).
pub async fn api_closed(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ClosedSummary>>, Error> {
    require_identity(&headers, &state)?;
    let tasks = task::list_closed(&state.db)?;
    Ok(Json(tasks.into_iter().map(ClosedSummary::from).collect()))
}

/// Remove a task that is over (`cancelled` / `dropped` / `released`) with the
/// subtasks that pointed at it. Anything else is 409: the row stays auditable
/// until a person calls it off.
pub async fn api_delete_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<task::Deleted>, Error> {
    require_human_mutation(&headers, &state)?;
    Ok(Json(task::delete(&state.db, &id)?))
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
            release_level: ReleaseLevel::parse_optional(body.release_level.as_deref())?,
            depends_on: body.depends_on,
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
        pending_merges: task::pending_merges(&state.db)?
            .into_iter()
            .map(PendingMerge::from)
            .collect(),
        pending_releases: task::pending_releases(&state.db)?
            .into_iter()
            .map(PendingRelease::from)
            .collect(),
        pending_reviews: summaries(task::pending_reviews(&state.db)?),
        unreviewed: summaries(task::unreviewed(&state.db)?),
        releasable: task::releasable(&state.db)?,
        stuck: task::stuck(&state.db, state.clock.now(), &state.stuck)?,
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

/// Reconciliation: issue the release of one product by hand. The ordinary flow
/// issues it at the landing.
pub async fn api_release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ReleaseBody>,
) -> Result<Response, Error> {
    require_human_mutation(&headers, &state)?;
    let issued = task::issue_release(&state.db, &body.product_id, state.clock.now())?;
    Ok((StatusCode::CREATED, Json(card(&state.db, issued)?)).into_response())
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

/// The result of a rescan, with whether this request walked or took the result
/// of a walk that finished after it arrived.
#[derive(Debug, Serialize)]
pub struct RescanAnswer {
    pub walked: bool,
    #[serde(flatten)]
    pub derived: product::Derived,
}

/// Walk the project tree now and make the catalogue equal it. Only with a
/// derived catalogue; a curated one answers 409 `catalogue_not_derived`.
/// Rescans are serialised, and a request that arrived while another was walking
/// is answered with that walk's result rather than walking again.
pub async fn api_rescan_products(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RescanAnswer>, Error> {
    require_human_mutation(&headers, &state)?;
    Ok(Json(rescan(&state)?))
}

/// The rescan itself, shared by the route and the MCP tool.
///
/// # Errors
/// `Error::Precondition` (`catalogue_not_derived`) without a tree; the walk's
/// or the database's error otherwise.
pub fn rescan(state: &AppState) -> Result<RescanAnswer, Error> {
    let Some(root) = state.projects_dir.clone() else {
        return Err(Error::Precondition {
            code: "catalogue_not_derived",
            message: "APP_PROJECTS_DIR is not set, so the catalogue is curated over the API and \
                      there is no project tree to rescan"
                .into(),
        });
    };
    rescan_with(state, || {
        product::derive_from_tree(&state.db, &root, state.clock.now())
    })
}

/// The serialisation around a walk, with the walk injected so it can be measured.
///
/// Requests queue on the gate. One that arrived while another was walking takes
/// that walk's result (`walked: false`): the tree it would have read is the tree
/// that walk read, at or after the moment it asked.
///
/// # Errors
/// Whatever `walk` returns.
pub fn rescan_with(
    state: &AppState,
    walk: impl FnOnce() -> Result<product::Derived, Error>,
) -> Result<RescanAnswer, Error> {
    let arrived = std::time::Instant::now();
    let mut gate = state
        .rescan
        .lock()
        .map_err(|_| Error::Io("rescan lock poisoned".into()))?;
    if let Some((finished, derived)) = &gate.finished
        && *finished >= arrived
    {
        return Ok(RescanAnswer {
            walked: false,
            derived: derived.clone(),
        });
    }
    let derived = walk()?;
    gate.finished = Some((std::time::Instant::now(), derived.clone()));
    Ok(RescanAnswer {
        walked: true,
        derived,
    })
}

pub async fn worker_claim(
    State(state): State<AppState>,
    Json(body): Json<ClaimBody>,
) -> Result<Response, Error> {
    let kinds = body
        .kinds
        .iter()
        .map(|raw| TaskKind::parse(raw))
        .collect::<Result<Vec<TaskKind>, Error>>()?;
    let leased = match body.idempotency_key.as_deref() {
        Some(key) => task::claim_idempotently(
            &state.db,
            &body.worker,
            &kinds,
            key,
            state.clock.now(),
            state.claim_ttl_secs,
        ),
        None => task::claim(
            &state.db,
            &body.worker,
            &kinds,
            state.clock.now(),
            state.claim_ttl_secs,
        ),
    }?;
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
    Json(body): Json<ReportBody>,
) -> Result<Json<TaskCard>, Error> {
    let outcome = ReportOutcome::parse_optional(body.outcome.as_deref())?;
    let reported = task::report(
        &state.db,
        &body.claim_id,
        &body.commit_sha,
        &body.verification,
        &body.checks,
        outcome,
        body.release_tag.as_deref(),
        state.clock.now(),
    )?;
    Ok(Json(card(&state.db, reported)?))
}

/// Hand a live claim back. The task returns to `ready` with the lease columns
/// cleared and the reason on `verification`; a claim that is not live answers
/// 409 with code `claim_not_live` and changes nothing.
pub async fn worker_claim_release(
    State(state): State<AppState>,
    Json(body): Json<ClaimReleaseBody>,
) -> Result<Json<TaskCard>, Error> {
    let released = task::release_claim(&state.db, &body.claim_id, &body.reason, state.clock.now())?;
    Ok(Json(card(&state.db, released)?))
}

/// A worker appends one run to the haystack. Same boundary as the other
/// `/worker/*` routes. 201 with the row when it was written, 200 with the row
/// already there when `(claim_id, attempt, source)` was seen before.
pub async fn worker_runs(
    State(state): State<AppState>,
    Json(body): Json<NewRun>,
) -> Result<Response, Error> {
    let (run, created) = runs::append(&state.db, &body, state.clock.now())?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(run)).into_response())
}

/// A person (or the rescue acting as one) leaves a note on a task. The source
/// is `rescue` whatever the body says: this door is the rescue's.
pub async fn api_runs_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<NewRun>,
) -> Result<Response, Error> {
    require_human_mutation(&headers, &state)?;
    body.source = Some(runs::Source::Rescue);
    let (run, created) = runs::append(&state.db, &body, state.clock.now())?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(run)).into_response())
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    #[serde(default)]
    pub since: i64,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub task_id: Option<String>,
}

/// Read the haystack forward from a watermark: `{ runs, next }`, `next` being
/// the `since` of the following page while one exists.
pub async fn api_runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RunsQuery>,
) -> Result<Json<runs::Page>, Error> {
    require_identity(&headers, &state)?;
    Ok(Json(runs::list(
        &state.db,
        query.since,
        query.limit,
        query.task_id.as_deref(),
    )?))
}

/// The review's own completion route.
///
/// It is not `/worker/report` because the two contracts differ: a review answers
/// with a verdict rather than a commit, carries no checks to gate on, and a
/// `request_changes` is a success — the reviewer did their job and the answer is
/// "not yet". The `claim_id` still binds the answer to the leased review.
pub async fn worker_review_report(
    State(state): State<AppState>,
    Json(body): Json<ReviewReportBody>,
) -> Result<Json<TaskCard>, Error> {
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
