#![allow(clippy::needless_pass_by_value)]
//! Ideas are Markdown notes that may or may not become tasks. They carry no
//! lifecycle or execution target; `revision` rejects edits based on a stale read.
use crate::{AppState, Error, format_z, task};
use serde_json::{Value, json};

const COLLECTION: &str = "idea";

fn validate(v: &Value, allowed: &[&str]) -> Result<(), Error> {
    let fields = v
        .as_object()
        .ok_or_else(|| Error::Invalid("idea fields must be an object".into()))?;
    for (key, value) in fields {
        if !allowed.contains(&key.as_str()) {
            return Err(Error::Invalid(format!(
                "unknown or immutable idea field: {key}"
            )));
        }
        let valid = match key.as_str() {
            "title" => value.as_str().is_some_and(|t| !t.trim().is_empty()),
            "body" | "execution_target" => value.is_string(),
            "product_id" => {
                value.is_null()
                    || value
                        .as_str()
                        .is_some_and(|p| crate::product::check_product_id(key, p).is_ok())
            }
            "expected_revision" => value.is_u64(),
            _ => true,
        };
        if !valid {
            return Err(Error::Invalid(format!("invalid idea field: {key}")));
        }
    }
    Ok(())
}

fn apply(idea: &mut Value, patch: &Value) {
    for key in ["title", "body", "product_id"] {
        if let Some(value) = patch.get(key) {
            idea[key] = value.clone();
        }
    }
}

fn revision(idea: &Value) -> u64 {
    idea["revision"].as_u64().unwrap_or(0)
}

fn archived(idea: &Value) -> bool {
    idea["archived"] == true
}

fn touch(s: &AppState, idea: &mut Value) {
    idea["revision"] = json!(revision(idea) + 1);
    idea["updated_at"] = json!(format_z(s.clock.now()));
}

fn writable(idea: &Value) -> Result<(), Error> {
    if archived(idea) {
        return Err(Error::Conflict("idea is archived and read-only".into()));
    }
    Ok(())
}

/// Hand-written notes may omit the revision; they read as revision 0.
fn project(mut idea: Value) -> Value {
    idea["revision"] = json!(revision(&idea));
    idea
}

pub fn create(s: &AppState, v: Value) -> Result<Value, Error> {
    validate(&v, &["title", "body", "product_id"])?;
    if v.get("title").is_none() {
        return Err(Error::Invalid("title is required".into()));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = format_z(s.clock.now());
    let mut idea = json!({"id":id,"title":"","body":"","product_id":null,"created_at":now,"updated_at":now,"revision":1,"archived":false,"archived_at":null,"task_id":null,"promoted_at":null});
    apply(&mut idea, &v);
    s.store.create(COLLECTION, &id, idea)
}

/// Newest update first; equal timestamps keep a stable id order.
pub fn list(s: &AppState, archived_only: bool) -> Result<Vec<Value>, Error> {
    let mut ideas = s.store.list(COLLECTION)?;
    ideas.retain(|idea| archived(idea) == archived_only);
    ideas.sort_by(|a, b| {
        task::string(b, "updated_at")
            .cmp(task::string(a, "updated_at"))
            .then_with(|| task::string(a, "id").cmp(task::string(b, "id")))
    });
    Ok(ideas
        .into_iter()
        .map(|idea| summary(&project(idea)))
        .collect())
}

pub fn get(s: &AppState, id: &str) -> Result<Value, Error> {
    s.store.get(COLLECTION, id).map(project)
}

pub fn update(s: &AppState, id: &str, v: Value) -> Result<Value, Error> {
    validate(&v, &["expected_revision", "title", "body", "product_id"])?;
    let expected = v["expected_revision"]
        .as_u64()
        .ok_or_else(|| Error::Invalid("expected_revision is required".into()))?;
    s.store.update(COLLECTION, id, |idea| {
        writable(idea)?;
        let current = revision(idea);
        if current != expected {
            return Err(Error::Conflict(format!(
                "idea changed: expected revision {expected}, current revision {current}; read it again and reapply the edit"
            )));
        }
        apply(idea, &v);
        touch(s, idea);
        Ok(())
    })
}

/// Idempotent; archived ideas stay readable but no longer accept edits.
pub fn archive(s: &AppState, id: &str) -> Result<Value, Error> {
    s.store.update(COLLECTION, id, |idea| {
        if !archived(idea) {
            idea["archived"] = json!(true);
            idea["archived_at"] = json!(format_z(s.clock.now()));
            touch(s, idea);
        }
        Ok(())
    })
}

/// Create the idea's draft task, or return the one an earlier attempt created.
/// The task id derives from the idea, so a retry after a stop between the two
/// file writes links the existing task instead of creating another.
pub fn promote(s: &AppState, id: &str, v: Value) -> Result<Value, Error> {
    validate(&v, &["title", "body", "product_id", "execution_target"])?;
    s.store.transaction(|a| {
        let mut idea = a.get(COLLECTION, id)?;
        writable(&idea)?;
        let task_id = format!("idea-{id}");
        let task = match a.get("tasks", &task_id) {
            Ok(existing) if existing["idea_id"] == id => existing,
            Ok(_) => {
                return Err(Error::Conflict(format!(
                    "task {task_id} is not linked to this idea"
                )));
            }
            Err(Error::NotFound(_)) => {
                let mut fields = json!({"id":task_id,"title":idea["title"],"body":idea["body"],"product_id":idea["product_id"]});
                for (key, value) in v.as_object().expect("validated object") {
                    fields[key] = value.clone();
                }
                let (_, mut record) = task::new_record(s, &fields)?;
                record["idea_id"] = json!(id);
                a.create("tasks", &task_id, record)?
            }
            Err(error) => return Err(error),
        };
        if idea["task_id"] != task_id {
            idea["task_id"] = json!(task_id);
            idea["promoted_at"] = json!(format_z(s.clock.now()));
            touch(s, &mut idea);
            idea = a.put(COLLECTION, id, idea)?;
        }
        Ok(json!({"idea":project(idea),"task":task}))
    })
}

/// List and receipt projection: everything except the body.
#[must_use]
pub fn summary(idea: &Value) -> Value {
    let keys = [
        "id",
        "title",
        "product_id",
        "created_at",
        "updated_at",
        "revision",
        "archived",
        "archived_at",
        "task_id",
        "promoted_at",
    ];
    Value::Object(
        keys.into_iter()
            .filter_map(|k| idea.get(k).map(|v| (k.into(), v.clone())))
            .collect(),
    )
}
