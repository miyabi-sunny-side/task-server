#![allow(clippy::needless_pass_by_value)]
//! Append-only session haystack with durable reading receipts.
use crate::{AppState, Error, format_z, task::string};
use serde_json::{Value, json};
fn trim(s: &str) -> (String, bool) {
    let mut end = s.len().min(8192);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].into(), s.len() > end)
}
pub fn append(s: &AppState, mut v: Value, rescue: bool) -> Result<Value, Error> {
    if !v.is_object() {
        return Err(Error::Invalid("run must be an object".into()));
    }
    if rescue {
        v["source"] = json!("rescue");
    }
    if string(&v, "source").is_empty() {
        return Err(Error::Invalid("source is required".into()));
    }
    s.store.transaction(|a| {
        let all = a.list("runs")?;
        if !v["claim_id"].is_null()
            && !v["attempt"].is_null()
            && let Some(old) = all.iter().find(|r| {
                r["claim_id"] == v["claim_id"]
                    && r["attempt"] == v["attempt"]
                    && r["source"] == v["source"]
            })
        {
            return Ok(old.clone());
        }
        let id = all
            .iter()
            .filter_map(|r| r["id"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        v["id"] = json!(id);
        if v["at"].is_null() {
            v["at"] = json!(format_z(s.clock.now()));
        }
        if v["product_id"].is_null() {
            v["product_id"] = match v["task_id"].as_str() {
                Some(id) => match a.get("tasks", id) {
                    Ok(t) => t["product_id"].clone(),
                    Err(Error::NotFound(_)) => Value::Null,
                    Err(e) => return Err(e),
                },
                None => Value::Null,
            };
        }
        let mut truncated = false;
        for k in ["stdout_tail", "stderr_tail", "note"] {
            if let Some(text) = v[k].as_str() {
                let (text, cut) = trim(text);
                v[k] = json!(text);
                truncated |= cut;
            }
        }
        v["truncated"] = json!(truncated);
        v["read_at"] = Value::Null;
        v["read_note"] = Value::Null;
        a.create("runs", &id.to_string(), v)
    })
}
pub fn read(s: &AppState, id: &str, v: Value) -> Result<Value, Error> {
    s.store.update("runs", id, |r| {
        if r["read_at"].is_null() {
            r["read_at"] = json!(format_z(s.clock.now()));
            let (note, cut) = trim(v["note"].as_str().or(v["read_note"].as_str()).unwrap_or(""));
            r["read_note"] = json!(note);
            if cut {
                r["truncated"] = json!(true);
            }
        }
        Ok(())
    })
}
