use crate::{AppState, Error, product, runs, task};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn identity(h: &HeaderMap, s: &AppState) -> Result<String, Error> {
    ["x-auth-user", "tailscale-user-login"]
        .iter()
        .find_map(|k| h.get(*k).and_then(|v| v.to_str().ok()))
        .or(s.dev_identity.as_deref())
        .filter(|v| !v.trim().is_empty())
        .map(str::to_owned)
        .ok_or(Error::Unauthorized)
}
fn mutation(h: &HeaderMap, s: &AppState) -> Result<(), Error> {
    identity(h, s)?;
    if h.get("x-csrf-token").and_then(|v| v.to_str().ok()) == Some(s.csrf_token.as_str())
        && !s.csrf_token.is_empty()
    {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}
pub async fn healthz() -> &'static str {
    "ok\n"
}
pub async fn api_health() -> Json<Value> {
    Json(json!({"status":"ok"}))
}
pub async fn api_session(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>, Error> {
    Ok(Json(
        json!({"user":identity(&h,&s)?,"csrf_token":s.csrf_token}),
    ))
}
pub async fn api_tasks(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<BTreeMap<String, String>>,
) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    let mut ts = if q.get("archived").is_some_and(|v| v == "true") {
        s.store
            .list("tasks")?
            .into_iter()
            .filter(|t| t["archived"] == true)
            .collect()
    } else {
        task::list(&s, q.get("status").map(String::as_str))?
    };
    if let Some(p) = q.get("product_id") {
        ts.retain(|t| t["product_id"] == *p);
    }
    Ok(Json(json!(
        ts.iter().map(task::summary).collect::<Vec<_>>()
    )))
}
pub async fn api_task(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    Ok(Json(task::card(&s, &id)?))
}
pub async fn api_create_task(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<Value>,
) -> Result<(StatusCode, Json<Value>), Error> {
    mutation(&h, &s)?;
    Ok((StatusCode::CREATED, Json(task::create(&s, v)?)))
}
pub async fn api_patch_task(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> Result<Json<Value>, Error> {
    mutation(&h, &s)?;
    task::patch(&s, &id, v)?;
    Ok(Json(task::card(&s, &id)?))
}
pub async fn api_delete_task(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    mutation(&h, &s)?;
    task::delete(&s, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn api_set_status(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> Result<Json<Value>, Error> {
    mutation(&h, &s)?;
    task::set_status(&s, &id, task::string(&v, "status"))?;
    Ok(Json(task::card(&s, &id)?))
}
fn history(s: &AppState, done: bool) -> Result<Value, Error> {
    task::sweep(s)?;
    let mut ts = s.store.list("tasks")?;
    ts.retain(|t| {
        if done {
            t["archived"] != true && t["status"] == "done"
        } else {
            matches!(task::string(t, "status"), "done" | "cancelled" | "dropped")
        }
    });
    ts.sort_by(|a, b| task::string(b, "closed_at").cmp(task::string(a, "closed_at")));
    Ok(json!(ts.iter().map(task::summary).collect::<Vec<_>>()))
}
pub async fn api_done(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    Ok(Json(history(&s, true)?))
}
pub async fn api_closed(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    Ok(Json(history(&s, false)?))
}
pub async fn api_control(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    let ts = task::list(&s, Some("blocked"))?;
    let stuck=ts.iter().map(|t|json!({"task_id":t["id"],"kind":t["kind"],"status":"blocked","since":t["updated_at"],"reason":"blocked"})).collect::<Vec<_>>();
    Ok(Json(
        json!({"mergeable":[],"pending_merges":[],"pending_releases":[],"pending_reviews":[],"unreviewed":[],"releasable":[],"stuck":stuck}),
    ))
}
pub async fn api_products(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    Ok(Json(json!(
        s.store
            .list("products")?
            .iter()
            .map(product::summary)
            .collect::<Vec<_>>()
    )))
}
pub async fn api_product(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    Ok(Json(s.store.get("products", &id)?))
}
pub async fn api_put_product(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> Result<Json<Value>, Error> {
    mutation(&h, &s)?;
    Ok(Json(product::put(&s, &id, v)?))
}
pub async fn api_rescan_products(
    State(s): State<AppState>,
    h: HeaderMap,
) -> Result<Json<Value>, Error> {
    mutation(&h, &s)?;
    Ok(Json(product::rescan(&s)?))
}
pub async fn retired() -> Error {
    Error::Gone
}
pub async fn api_not_found() -> Error {
    Error::NotFound("unknown API route".into())
}
pub async fn worker_claim(
    State(s): State<AppState>,
    Json(v): Json<Value>,
) -> Result<Response, Error> {
    Ok(match task::claim(&s, task::string(&v, "worker"))? {
        Some(v) => Json(v).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}
pub async fn worker_heartbeat(
    State(s): State<AppState>,
    Json(v): Json<Value>,
) -> Result<Json<Value>, Error> {
    Ok(Json(task::heartbeat(&s, task::string(&v, "claim_id"))?))
}
pub async fn worker_report(
    State(s): State<AppState>,
    Json(v): Json<Value>,
) -> Result<Json<Value>, Error> {
    Ok(Json(task::report(&s, v)?))
}
pub async fn worker_runs(
    State(s): State<AppState>,
    Json(v): Json<Value>,
) -> Result<Json<Value>, Error> {
    Ok(Json(runs::append(&s, v, false)?))
}
pub async fn api_runs_post(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<Value>,
) -> Result<Json<Value>, Error> {
    mutation(&h, &s)?;
    Ok(Json(runs::append(&s, v, true)?))
}
fn filtered_runs(s: &AppState, q: &BTreeMap<String, String>) -> Result<Vec<Value>, Error> {
    task::sweep(s)?;
    let mut rs = s.store.list("runs")?;
    rs.retain(|r| {
        ["task_id", "product_id", "source"]
            .iter()
            .all(|k| q.get(*k).is_none_or(|v| r[*k] == *v))
            && (q.get("unread").is_none_or(|v| v != "1" && v != "true") || r["read_at"].is_null())
    });
    rs.sort_by_key(|r| r["id"].as_u64().unwrap_or(0));
    Ok(rs)
}
pub async fn api_runs(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<BTreeMap<String, String>>,
) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    let since = q
        .get("since")
        .map(|v| v.parse::<u64>())
        .transpose()
        .map_err(|_| Error::Invalid("invalid since".into()))?
        .unwrap_or(0);
    let limit = q
        .get("limit")
        .map(|v| v.parse::<usize>())
        .transpose()
        .map_err(|_| Error::Invalid("invalid limit".into()))?
        .unwrap_or(100)
        .clamp(1, 1000);
    let mut rs = filtered_runs(&s, &q)?
        .into_iter()
        .filter(|r| r["id"].as_u64().unwrap_or(0) > since)
        .collect::<Vec<_>>();
    let has_more = rs.len() > limit;
    rs.truncate(limit);
    let next = if has_more {
        rs.last().and_then(|r| r["id"].as_u64())
    } else {
        None
    };
    Ok(Json(json!({"runs":rs,"next":next})))
}
pub async fn api_runs_next(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<BTreeMap<String, String>>,
) -> Result<Response, Error> {
    identity(&h, &s)?;
    let mut rs = filtered_runs(&s, &q)?;
    rs.retain(|r| r["read_at"].is_null());
    rs.sort_by(|a, b| {
        task::string(a, "at")
            .cmp(task::string(b, "at"))
            .then_with(|| a["id"].as_u64().cmp(&b["id"].as_u64()))
    });
    Ok(rs.into_iter().next().map_or_else(
        || StatusCode::NO_CONTENT.into_response(),
        |r| Json(r).into_response(),
    ))
}
pub async fn api_run_read(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    Ok(Json(runs::read(
        &s,
        &id,
        body.map_or_else(|| json!({}), |Json(v)| v),
    )?))
}
pub async fn worker_snapshot(State(s): State<AppState>) -> Result<Json<Value>, Error> {
    task::sweep(&s)?;
    Ok(Json(s.store.transaction(|a| {
        crate::report::recover(a)?;
        let mut result = json!({});
        for c in ["tasks", "products", "runs", "archive", "claim_receipts"] {
            result[c] = json!(a.list(c)?);
        }
        Ok(result)
    })?))
}

pub async fn api_run(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    identity(&h, &s)?;
    Ok(Json(crate::report::get(&s, &id)?))
}
