//! Execution-scoped handoff values in the task document, guarded by claim and revision.
use crate::{AppState, Error, format_z, task};
use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub claim_id: String,
    pub expected_revision: u64,
    #[serde(default)]
    pub set: Map<String, Value>,
    #[serde(default)]
    pub delete_keys: Vec<String>,
}

pub(crate) fn begin(t: &mut Value, now: &str) -> Result<(), Error> {
    if t["execution_checkpoints"].is_null() {
        t["execution_checkpoints"] = json!([]);
    }
    let execution_id = t["claim_id"].clone();
    let checkpoints = t["execution_checkpoints"]
        .as_array_mut()
        .ok_or_else(|| Error::Invalid("execution_checkpoints must be an array".into()))?;
    checkpoints
        .push(json!({"execution_id":execution_id,"revision":0,"updated_at":now,"values":{}}));
    Ok(())
}

pub fn get(s: &AppState, id: &str, execution_id: Option<&str>) -> Result<Value, Error> {
    task::sweep(s)?;
    s.store.transaction(|a| {
        let mut t = a.get("tasks", id)?;
        if t["claim_id"].is_string() && !has_current(&t) {
            let now = t["claimed_at"]
                .as_str()
                .unwrap_or(&format_z(s.clock.now()))
                .to_owned();
            begin(&mut t, &now)?;
        }
        let checkpoints: Vec<_> = t["execution_checkpoints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| execution_id.is_none_or(|id| c["execution_id"] == id))
            .cloned()
            .collect();
        if execution_id.is_some() && checkpoints.is_empty() {
            return Err(Error::NotFound("execution checkpoint".into()));
        }
        Ok(json!({"task_id":id,"active_claim_id":t["claim_id"],"checkpoints":checkpoints}))
    })
}

fn has_current(t: &Value) -> bool {
    t["execution_checkpoints"]
        .as_array()
        .is_some_and(|items| items.iter().any(|c| c["execution_id"] == t["claim_id"]))
}

fn validate(patch: &Patch) -> Result<(), Error> {
    if patch.set.len() > 64 || patch.delete_keys.len() > 64 {
        return Err(Error::Invalid(
            "checkpoint patch supports at most 64 keys".into(),
        ));
    }
    for key in patch.set.keys().chain(patch.delete_keys.iter()) {
        if key.trim().is_empty() || key.len() > 128 {
            return Err(Error::Invalid(
                "checkpoint keys must contain 1..128 bytes".into(),
            ));
        }
    }
    if patch
        .delete_keys
        .iter()
        .any(|key| patch.set.contains_key(key))
    {
        return Err(Error::Invalid(
            "cannot set and delete the same checkpoint key".into(),
        ));
    }
    Ok(())
}

pub fn update(s: &AppState, id: &str, patch: Patch) -> Result<Value, Error> {
    validate(&patch)?;
    s.store.transaction(|a| {
        let now = format_z(s.clock.now());
        let mut t = task::claimed(a, &patch.claim_id, &now)?;
        if t["id"] != id {
            return Err(Error::Conflict("claim belongs to a different task".into()));
        }
        // A lease created by an older server starts at revision zero on first use.
        if !has_current(&t) {
            begin(&mut t, &now)?;
        }
        let checkpoint = t["execution_checkpoints"]
            .as_array_mut()
            .and_then(|items| {
                items
                    .iter_mut()
                    .find(|c| c["execution_id"] == patch.claim_id)
            })
            .ok_or_else(|| Error::Conflict("execution checkpoint is missing".into()))?;
        if checkpoint["revision"].as_u64() != Some(patch.expected_revision) {
            return Err(Error::Conflict(
                "checkpoint revision changed; read and reapply only intended keys".into(),
            ));
        }
        let values = checkpoint["values"]
            .as_object_mut()
            .ok_or_else(|| Error::Invalid("checkpoint values must be an object".into()))?;
        for key in patch.delete_keys {
            values.remove(&key);
        }
        values.extend(patch.set);
        if values.len() > 64 || serde_json::to_vec(values)?.len() > 32768 {
            return Err(Error::Invalid(
                "checkpoint values exceed 64 keys or 32768 JSON bytes".into(),
            ));
        }
        checkpoint["revision"] = json!(
            patch
                .expected_revision
                .checked_add(1)
                .ok_or_else(|| Error::Conflict("checkpoint revision exhausted".into()))?
        );
        checkpoint["updated_at"] = json!(now);
        let result = checkpoint.clone();
        a.put("tasks", id, t)?;
        Ok(result)
    })
}
