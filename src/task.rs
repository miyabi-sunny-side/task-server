#![allow(clippy::needless_pass_by_value)]
//! Task lifecycle and execution leases; milestone evidence is independent of status.
use crate::{AppState, Error, format_z, ledger::StoreAccess};
use serde_json::{Value, json};
const STATUSES: &[&str] = &[
    "draft",
    "ready",
    "wip",
    "blocked",
    "done",
    "cancelled",
    "dropped",
];
#[must_use]
pub fn string<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
pub fn check_status(s: &str) -> Result<(), Error> {
    if STATUSES.contains(&s) {
        Ok(())
    } else {
        Err(Error::Invalid(format!("unknown status {s}")))
    }
}
fn active(t: &Value) -> bool {
    t["archived"] != true && (t["kind"].is_null() || t["kind"] == "normal")
}
fn closed(s: &str) -> bool {
    matches!(s, "done" | "cancelled" | "dropped")
}
fn required(v: &Value, k: &str) -> Result<String, Error> {
    let s = string(v, k);
    if s.trim().is_empty() {
        Err(Error::Invalid(format!("{k} is required")))
    } else {
        Ok(s.into())
    }
}
pub fn create(s: &AppState, v: Value) -> Result<Value, Error> {
    let id = v["id"]
        .as_str()
        .map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned);
    let title = required(&v, "title")?;
    if id.trim().is_empty() || id.contains('/') || id == "." || id == ".." {
        return Err(Error::Invalid("id must be one path segment".into()));
    }
    if let Some(p) = v["product_id"].as_str() {
        crate::product::check_product_id("product_id", p)?;
    }
    let now = format_z(s.clock.now());
    let mut t = json!({"id":id,"title":title,"body":"","status":"draft","kind":"normal","priority":0,"product_id":null,"depends_on":null,"blocked_by":null,"branch":null,"commit_sha":null,"verification":null,"summary":null,"checks":[],"milestones":[],"milestone_history":[],"claim_id":null,"claimed_by":null,"lease_expires_at":null,"created_at":now,"updated_at":now,"done_at":null,"closed_at":null});
    for k in [
        "body",
        "product_id",
        "priority",
        "depends_on",
        "release_level",
    ] {
        if let Some(value) = v.get(k) {
            t[k] = value.clone();
        }
    }
    s.store.create("tasks", &id, t)
}
pub(crate) fn invalidate(t: &mut Value, new: &Value) {
    if &t["commit_sha"] != new {
        let old = t["milestones"].as_array().cloned().unwrap_or_default();
        let (expired, keep): (Vec<_>, Vec<_>) = old.into_iter().partition(|m| {
            matches!(
                string(m, "name"),
                "verified" | "reviewed" | "merged" | "released"
            )
        });
        let mut history = t["milestone_history"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        history.extend(expired);
        t["milestone_history"] = json!(history);
        t["milestones"] = json!(keep);
        t["commit_sha"] = new.clone();
    }
}
pub(crate) fn milestones(
    t: &mut Value,
    v: &Value,
    now: &str,
    report_id: Option<u64>,
) -> Result<(), Error> {
    let Some(items) = v.as_array() else {
        return Err(Error::Invalid("milestones must be an array".into()));
    };
    let mut result = t["milestones"].as_array().cloned().unwrap_or_default();
    for m in items {
        if !["implemented", "verified", "reviewed", "merged", "released"]
            .contains(&string(m, "name"))
        {
            return Err(Error::Invalid("unknown milestone".into()));
        }
        let mut m = m.clone();
        if m["at"].is_null() {
            m["at"] = json!(now);
        }
        if m["commit_sha"].is_null() {
            m["commit_sha"] = t["commit_sha"].clone();
        }
        if m["evidence"].is_null() && (report_id.is_none() || m["report_id"].as_u64() != report_id)
        {
            return Err(Error::Invalid("milestone evidence is required".into()));
        }
        if m["commit_sha"] != t["commit_sha"] {
            return Err(Error::Invalid(
                "milestone commit must match task commit".into(),
            ));
        }
        result.push(m);
    }
    t["milestones"] = json!(result);
    Ok(())
}
pub fn patch(s: &AppState, id: &str, v: Value) -> Result<Value, Error> {
    s.store.transaction(|a| {
        expire(a, &format_z(s.clock.now()))?;
        a.update("tasks", id, |t| {
            if v.get("status").is_some() {
                return Err(Error::Invalid("use status endpoint".into()));
            }
            if t["claim_id"].is_string() {
                return Err(Error::Conflict("task has an execution lease".into()));
            }
            if let Some(p) = v["product_id"].as_str() {
                crate::product::check_product_id("product_id", p)?;
            }
            if v.get("title").is_some() {
                required(&v, "title")?;
            }
            if let Some(commit) = v.get("commit_sha") {
                invalidate(t, commit);
            }
            for k in [
                "title",
                "body",
                "product_id",
                "priority",
                "depends_on",
                "branch",
                "verification",
                "summary",
                "checks",
                "release_level",
            ] {
                if let Some(value) = v.get(k) {
                    t[k] = value.clone();
                }
            }
            let now = format_z(s.clock.now());
            if let Some(m) = v.get("milestones") {
                milestones(t, m, &now, None)?;
            }
            t["updated_at"] = json!(now);
            Ok(())
        })
    })
}
fn ready_gate(a: &StoreAccess<'_>, t: &Value) -> Result<(), Error> {
    let id = required(t, "product_id")?;
    let p = a.get("products", &id).map_err(|e| match e {
        Error::NotFound(_) => Error::Conflict("product_not_catalogued".into()),
        e => e,
    })?;
    if p["archived"] == true || p["archived_at"].is_string() {
        return Err(Error::Conflict("product_archived".into()));
    }
    Ok(())
}
pub fn set_status(s: &AppState, id: &str, status: &str) -> Result<Value, Error> {
    check_status(status)?;
    s.store.transaction(|a| {
        expire(a, &format_z(s.clock.now()))?;
        let mut t = a.get("tasks", id)?;
        if !active(&t) {
            return Err(Error::Gone);
        }
        if status == "ready" {
            ready_gate(a, &t)?;
        }
        if status == "wip" {
            return Err(Error::Conflict("wip requires worker claim".into()));
        }
        if t["claim_id"].is_string() && status != string(&t, "status") {
            t["claim_id"] = Value::Null;
            t["lease_expires_at"] = Value::Null;
        }
        t["status"] = json!(status);
        t["updated_at"] = json!(format_z(s.clock.now()));
        if status == "done" && t["done_at"].is_null() {
            t["done_at"] = t["updated_at"].clone();
        }
        if closed(status) {
            t["closed_at"] = t["updated_at"].clone();
        }
        if status == "ready" {
            t["blocked_by"] = Value::Null;
        }
        a.put("tasks", id, t)
    })
}
pub fn delete(s: &AppState, id: &str) -> Result<Value, Error> {
    s.store.transaction(|a| {
        expire(a, &format_z(s.clock.now()))?;
        let t = a.get("tasks", id)?;
        if !closed(string(&t, "status")) {
            return Err(Error::Conflict("only closed tasks may be deleted".into()));
        }
        a.remove("tasks", id)
    })
}
fn expire(a: &StoreAccess<'_>, now: &str) -> Result<(), Error> {
    crate::report::recover(a)?;
    for mut t in a.list("tasks")? {
        if t["status"] == "wip"
            && t["claim_id"].is_string()
            && string(&t, "lease_expires_at") <= now
        {
            t["status"] = json!("blocked");
            t["blocked_by"] = json!("worker");
            t["verification"] = json!("interrupted: execution lease expired");
            t["updated_at"] = json!(now);
            t["interrupted_claim_id"] = t["claim_id"].take();
            t["lease_expires_at"] = Value::Null;
            let id = required(&t, "id")?;
            a.put("tasks", &id, t)?;
        }
    }
    Ok(())
}
pub fn sweep(s: &AppState) -> Result<(), Error> {
    s.store.transaction(|a| expire(a, &format_z(s.clock.now())))
}
pub fn list(s: &AppState, status: Option<&str>) -> Result<Vec<Value>, Error> {
    if let Some(status) = status {
        check_status(status)?;
    }
    sweep(s)?;
    let mut ts = s.store.list("tasks")?;
    ts.retain(|t| active(t) && status.map_or(!closed(string(t, "status")), |x| t["status"] == x));
    for t in &mut ts {
        if let Some(dependency) = t["depends_on"].as_str() {
            match s.store.get("tasks", dependency) {
                Ok(dep) if dep["status"] != "done" => {
                    t["dependency_status"] = dep["status"].clone();
                }
                Ok(_) | Err(Error::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }
    ts.sort_by(|a, b| {
        b["priority"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["priority"].as_i64().unwrap_or(0))
            .then_with(|| string(a, "id").cmp(string(b, "id")))
    });
    Ok(ts)
}
pub fn card(s: &AppState, id: &str) -> Result<Value, Error> {
    sweep(s)?;
    let mut t = s.store.get("tasks", id)?;
    t["available_transitions"] = if active(&t) {
        json!(
            STATUSES
                .iter()
                .filter(|x| **x != "wip" && **x != string(&t, "status"))
                .collect::<Vec<_>>()
        )
    } else {
        json!([])
    };
    let rs = s
        .store
        .list("runs")?
        .into_iter()
        .filter(|r| r["task_id"] == id)
        .collect::<Vec<_>>();
    t["runs_count"] = json!(rs.len());
    t["runs_unread"] = json!(rs.iter().filter(|r| r["read_at"].is_null()).count());
    Ok(t)
}
fn envelope(t: Value) -> Value {
    json!({"claim_id":t["claim_id"],"lease_expires_at":t["lease_expires_at"],"task":t})
}
pub fn claim(s: &AppState, worker: &str) -> Result<Option<Value>, Error> {
    if worker.trim().is_empty() {
        return Err(Error::Invalid("worker is required".into()));
    }
    s.store.transaction(|a| {
        let now = format_z(s.clock.now());
        expire(a, &now)?;
        let mut ts = a.list("tasks")?;
        ts.sort_by(|a, b| {
            b["priority"]
                .as_i64()
                .unwrap_or(0)
                .cmp(&a["priority"].as_i64().unwrap_or(0))
                .then_with(|| string(a, "created_at").cmp(string(b, "created_at")))
        });
        for mut t in ts {
            if !active(&t) || t["status"] != "ready" {
                continue;
            }
            if let Err(error) = ready_gate(a, &t) {
                match error {
                    Error::Conflict(_) | Error::Invalid(_) => {
                        block_unclaimable(a, &mut t, &now, &error.to_string())?;
                        continue;
                    }
                    error => return Err(error),
                }
            }
            if let Some(dep) = t["depends_on"].as_str() {
                match a.get("tasks", dep) {
                    Ok(dependency) if dependency["status"] == "done" => {}
                    Ok(_) => continue,
                    Err(Error::NotFound(_)) => {
                        let reason = format!("dependency {dep} is missing");
                        block_unclaimable(a, &mut t, &now, &reason)?;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            t["status"] = json!("wip");
            t["claim_id"] = json!(uuid::Uuid::new_v4().to_string());
            t["claimed_by"] = json!(worker);
            t["claimed_at"] = json!(now);
            t["updated_at"] = json!(now);
            t["lease_expires_at"] = json!(format_z(
                s.clock.now()
                    + time::Duration::seconds(
                        i64::try_from(s.claim_ttl_secs)
                            .map_err(|_| Error::Invalid("lease TTL too large".into()))?
                    )
            ));
            let id = required(&t, "id")?;
            return Ok(Some(envelope(a.put("tasks", &id, t)?)));
        }
        Ok(None)
    })
}
pub(crate) fn claimed(a: &StoreAccess<'_>, id: &str, now: &str) -> Result<Value, Error> {
    expire(a, now)?;
    a.list("tasks")?
        .into_iter()
        .find(|t| t["claim_id"] == id && t["status"] == "wip")
        .ok_or_else(|| Error::Conflict("claim mismatch or expired".into()))
}
pub fn heartbeat(s: &AppState, id: &str) -> Result<Value, Error> {
    s.store.transaction(|a| {
        let mut t = claimed(a, id, &format_z(s.clock.now()))?;
        t["lease_expires_at"] = json!(format_z(
            s.clock.now()
                + time::Duration::seconds(
                    i64::try_from(s.claim_ttl_secs)
                        .map_err(|_| Error::Invalid("lease TTL too large".into()))?
                )
        ));
        let tid = required(&t, "id")?;
        Ok(envelope(a.put("tasks", &tid, t)?))
    })
}
pub fn report(s: &AppState, v: Value) -> Result<Value, Error> {
    if v.get("report_markdown").is_some() {
        return crate::report::submit(s, &v);
    }
    let claim = required(&v, "claim_id")?;
    let outcome = required(&v, "outcome")?;
    if !["done", "blocked"].contains(&outcome.as_str()) {
        return Err(Error::Invalid("outcome must be done or blocked".into()));
    }
    s.store.transaction(|a| {
        let now = format_z(s.clock.now());
        for old in a.list("tasks")? {
            if old["last_claim_id"] == claim {
                return if old["last_report"] == v {
                    Ok(old)
                } else {
                    Err(Error::Conflict(
                        "claim already reported with different result".into(),
                    ))
                };
            }
        }
        let mut t = claimed(a, &claim, &now)?;
        if let Some(commit) = v.get("commit_sha") {
            invalidate(&mut t, commit);
        }
        for k in ["verification", "summary", "checks"] {
            if let Some(value) = v.get(k) {
                t[k] = value.clone();
            }
        }
        if let Some(m) = v.get("milestones") {
            milestones(&mut t, m, &now, None)?;
        }
        t["report_id"] = Value::Null;
        t["last_report"] = v.clone();
        t["last_claim_id"] = t["claim_id"].take();
        t["lease_expires_at"] = Value::Null;
        t["status"] = json!(outcome);
        t["updated_at"] = json!(now);
        t["blocked_by"] = if outcome == "blocked" {
            json!("worker")
        } else {
            Value::Null
        };
        if outcome == "done" {
            if t["done_at"].is_null() {
                t["done_at"] = json!(now);
            }
            t["closed_at"] = json!(now);
        }
        let id = required(&t, "id")?;
        a.put("tasks", &id, t)
    })
}

/// Project only fields used by task list screens; the document remains intact.
#[must_use]
pub fn summary(t: &Value) -> Value {
    let keys = [
        "id",
        "title",
        "status",
        "kind",
        "product_id",
        "priority",
        "created_at",
        "updated_at",
        "done_at",
        "closed_at",
        "depends_on",
        "dependency_status",
        "blocked_by",
        "summary",
        "verification",
        "milestones",
        "archived",
        "commit_sha",
        "release_tag",
        "release_level",
    ];
    Value::Object(
        keys.into_iter()
            .filter_map(|k| t.get(k).map(|v| (k.into(), v.clone())))
            .collect(),
    )
}

fn block_unclaimable(
    a: &StoreAccess<'_>,
    t: &mut Value,
    now: &str,
    reason: &str,
) -> Result<(), Error> {
    t["status"] = json!("blocked");
    t["verification"] = json!(reason);
    t["blocked_by"] = json!("system");
    t["updated_at"] = json!(now);
    let id = required(t, "id")?;
    a.put("tasks", &id, t.clone())?;
    Ok(())
}
