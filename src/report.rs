//! A report's run document is also its durable, replayable completion intent.
//! Publish the original first; task references can then be recovered without
//! copying prose or accepting an expired, previously unsubmitted result.
use crate::{AppState, Error, format_z, ledger::StoreAccess, task};
use serde_json::{Value, json};

pub fn submit(s: &AppState, v: &Value) -> Result<Value, Error> {
    let raw = v["report_markdown"]
        .as_str()
        .filter(|x| !x.trim().is_empty())
        .ok_or_else(|| Error::Invalid("report_markdown is required".into()))?;
    let claim = task::string(v, "claim_id");
    if claim.trim().is_empty() || !matches!(task::string(v, "outcome"), "done" | "blocked") {
        return Err(Error::Invalid(
            "claim_id and outcome done/blocked are required".into(),
        ));
    }
    let mut request = v.clone();
    request
        .as_object_mut()
        .expect("validated report object")
        .remove("report_markdown");
    // One prose input. Old callers retain the separate legacy report path.
    if ["summary", "verification"]
        .iter()
        .any(|k| v.get(k).is_some())
    {
        return Err(Error::Invalid(
            "use report_markdown instead of summary/verification".into(),
        ));
    }
    validate_checks(v)?;
    s.store.transaction(|a| {
        recover(a)?;
        let all = a.list("runs")?;
        if let Some(old) = all.iter().find(|r| r["report_request"].is_object() && r["claim_id"] == claim) {
            if old["report_request"] != request || old["body"] != raw {
                return Err(Error::Conflict("claim already reported with different result".into()));
            }
            return a.get("tasks", task::string(old,"task_id"));
        }
        let now = format_z(s.clock.now());
        let t = task::claimed(a, claim, &now)?;
        let id = all.iter().filter_map(|r| r["id"].as_u64()).max().unwrap_or(0) + 1;
        let mut run = json!({"id":id,"source":"worker","attempt":1,"task_id":t["id"],"product_id":t["product_id"],"claim_id":claim,"at":now,"outcome":v["outcome"],"commit_sha":v.get("commit_sha").unwrap_or(&t["commit_sha"]),"checks":v.get("checks").cloned().unwrap_or_else(||json!([])),"body":raw,"read_at":null,"read_note":null,"report_request":request});
        if let Some(extra) = v.get("run") {
            if !extra.is_object() { return Err(Error::Invalid("run must be an object".into())); }
            for key in ["worker","model","agent_exit","agent_secs","stdout_tail","stderr_tail"] {
                if let Some(value) = extra.get(key) { run[key]=value.clone(); }
            }
        }
        // Validate every task change before the durable acceptance point.
        let updated = apply(t, &run)?;
        a.create("runs", &id.to_string(), run)?;
        a.put("tasks", task::string(&updated,"id"), updated.clone())
    })
}

fn validate_checks(v: &Value) -> Result<(), Error> {
    if let Some(checks) = v.get("checks") {
        let checks = checks
            .as_array()
            .ok_or_else(|| Error::Invalid("checks must be an array".into()))?;
        for check in checks {
            if task::string(check, "name").trim().is_empty()
                || check["exit_code"].as_i64().is_none()
            {
                return Err(Error::Invalid(
                    "checks require name and integer exit_code".into(),
                ));
            }
            if v["outcome"] == "done" && check["exit_code"] != 0 {
                return Err(Error::Invalid("done report contains failed checks".into()));
            }
        }
    }
    Ok(())
}

fn apply(mut t: Value, run: &Value) -> Result<Value, Error> {
    let v = &run["report_request"];
    if t["summary"].is_string()
        || t["verification"].is_string()
        || t["checks"]
            .as_array()
            .is_some_and(|checks| !checks.is_empty())
    {
        let mut history = t["legacy_completion"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        history.push(json!({"at":t["updated_at"],"claim_id":t["last_claim_id"],"commit_sha":t["commit_sha"],"summary":t["summary"].take(),"verification":t["verification"].take(),"checks":t["checks"].take()}));
        t["legacy_completion"] = json!(history);
        t["checks"] = json!([]);
    }
    let now = task::string(run, "at");
    if let Some(commit) = v.get("commit_sha") {
        task::invalidate(&mut t, commit);
    }
    if let Some(items) = v.get("milestones") {
        let mut items = items
            .as_array()
            .ok_or_else(|| Error::Invalid("milestones must be an array".into()))?
            .clone();
        for item in &mut items {
            if !item.is_object() {
                return Err(Error::Invalid("milestone must be an object".into()));
            }
            item["report_id"] = run["id"].clone();
        }
        task::milestones(&mut t, &json!(items), now, run["id"].as_u64())?;
    }
    t["report_id"] = run["id"].clone();
    let mut ids = t["report_ids"].as_array().cloned().unwrap_or_default();
    ids.push(run["id"].clone());
    t["report_ids"] = json!(ids);
    // Historical legacy prose/evidence stays intact; new prose lives only in run.body.
    t["last_claim_id"] = t["claim_id"].take();
    t["lease_expires_at"] = Value::Null;
    t["status"] = v["outcome"].clone();
    t["updated_at"] = json!(now);
    t["blocked_by"] = if v["outcome"] == "blocked" {
        json!("worker")
    } else {
        Value::Null
    };
    if v["outcome"] == "done" {
        if t["done_at"].is_null() {
            t["done_at"] = json!(now);
        }
        t["closed_at"] = json!(now);
    }
    Ok(t)
}

/// Run before task reads/mutations and lease expiry. Only an already accepted
/// intent for the task's current claim can complete it; old runs cannot rewind it.
pub fn recover(a: &StoreAccess<'_>) -> Result<(), Error> {
    for run in a.list("runs")? {
        if !run["report_request"].is_object() {
            continue;
        }
        let id = task::string(&run, "task_id");
        let t = match a.get("tasks", id) {
            Ok(t) => t,
            Err(Error::NotFound(_)) => continue,
            Err(e) => return Err(e),
        };
        if t["claim_id"].is_string() && t["claim_id"] == run["claim_id"] && t["status"] == "wip" {
            a.put("tasks", id, apply(t, &run)?)?;
        }
    }
    Ok(())
}

pub fn get(s: &AppState, id: &str) -> Result<Value, Error> {
    task::sweep(s)?;
    s.store.get("runs", id)
}
