use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Error;
use crate::product::{self, Product};
use crate::state::AppState;
use crate::task::{self, NewTask, Task, TaskKind, TaskPatch, TaskStatus};

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

/// The full task plus the transitions a human may actually press.
#[derive(Debug, Serialize)]
pub struct TaskCard {
    #[serde(flatten)]
    pub task: Task,
    pub available_transitions: Vec<TaskStatus>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimBody {
    pub worker: String,
}

#[derive(Debug, Deserialize)]
pub struct ReportBody {
    pub claim_id: String,
    pub commit_sha: String,
    pub verification: String,
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

fn require_identity(headers: &HeaderMap, state: &AppState) -> Result<String, Error> {
    let Some(name) = ingress_identity(headers, state) else {
        return Err(Error::Unauthorized);
    };
    if state.allowlist.iter().any(|allowed| allowed == name) {
        Ok(name.to_owned())
    } else {
        Err(Error::Unauthorized)
    }
}

fn origin_allowed(state: &AppState, origin: &str) -> bool {
    if state
        .allowed_origins
        .iter()
        .any(|allowed| allowed == origin)
    {
        return true;
    }
    state.dev_identity.is_some()
        && state.allowed_origins.is_empty()
        && (origin.starts_with("http://127.0.0.1") || origin.starts_with("http://localhost"))
}

fn require_human_mutation(headers: &HeaderMap, state: &AppState) -> Result<(), Error> {
    require_identity(headers, state)?;
    let Some(origin) = header_value(headers, "origin") else {
        return Err(Error::Unauthorized);
    };
    if !origin_allowed(state, origin) {
        return Err(Error::Forbidden);
    }
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
    let available_transitions = task::available_transitions(db, &task)?;
    Ok(TaskCard {
        task,
        available_transitions,
    })
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
    Ok(Json(tasks.into_iter().map(TaskSummary::from).collect()))
}

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
    let moved = task::set_status(&state.db, &id, to, state.clock.now())?;
    Ok(Json(card(&state.db, moved)?))
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
    let leased = task::claim(
        &state.db,
        &body.worker,
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
        state.clock.now(),
    )?;
    Ok(Json(card(&state.db, reported)?))
}

pub async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "API route not found\n")
}
