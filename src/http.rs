use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::error::Error;
use crate::state::AppState;
use crate::store::{self, ReportRequest};

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

#[derive(Debug, Default, Deserialize)]
pub struct ActionBody {
    pub bump: Option<String>,
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

pub async fn healthz() -> &'static str {
    "ok\n"
}

pub async fn api_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

pub async fn worker_claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ClaimBody>,
) -> Result<Response, Error> {
    require_worker(&headers, &state)?;
    match store::claim(&state, &body.worker)? {
        Some(lease) => Ok(Json(lease).into_response()),
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
) -> Result<Json<store::ReportOutcome>, Error> {
    require_worker(&headers, &state)?;
    let outcome = store::report(
        &state,
        &ReportRequest {
            claim_id: body.claim_id,
            commit_sha: body.commit_sha,
            verification: body.verification,
        },
    )?;
    Ok(Json(outcome))
}

pub async fn api_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<store::TaskSummary>>, Error> {
    require_identity(&headers, &state)?;
    Ok(Json(store::list_tasks(&state)?))
}

pub async fn api_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<store::TaskCard>, Error> {
    require_identity(&headers, &state)?;
    Ok(Json(store::get_task(&state, &id)?))
}

pub async fn api_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, action)): Path<(String, String)>,
    body: Option<Json<ActionBody>>,
) -> Result<Json<store::TaskCard>, Error> {
    require_human_mutation(&headers, &state)?;
    let bump = body.and_then(|Json(payload)| payload.bump);
    Ok(Json(store::apply_human_action(
        &state,
        &id,
        &action,
        bump.as_deref(),
    )?))
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

pub async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "API route not found\n")
}
