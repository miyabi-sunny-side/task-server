#![allow(clippy::needless_pass_by_value)]
use crate::{AppState, Error, format_z};
use serde_json::{Value, json};
pub fn check_product_id(_field: &str, id: &str) -> Result<(), Error> {
    let seg = id.split('/').collect::<Vec<_>>();
    if seg.len() != 2
        || seg.iter().any(|s| {
            s.is_empty()
                || *s == "."
                || *s == ".."
                || !s
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        })
    {
        Err(Error::Invalid("product id must be org/repo".into()))
    } else {
        Ok(())
    }
}
/// Explicit registration is create-only: it cannot overwrite or revive an ID.
pub fn put(s: &AppState, id: &str, v: Value) -> Result<Value, Error> {
    check_product_id("id", id)?;
    validate(&v)?;
    if v["repository"].as_str().is_none_or(|r| r.trim().is_empty()) {
        return Err(Error::Invalid("repository is required".into()));
    }
    let now = format_z(s.clock.now());
    let mut p = json!({"id":id,"repository":v["repository"],"description":"","local_path":null,"releases":null,"archived":false,"created_at":now,"updated_at":now});
    apply(&mut p, &v);
    s.store.create("products", id, p)
}

fn validate(v: &Value) -> Result<(), Error> {
    let fields = v
        .as_object()
        .ok_or_else(|| Error::Invalid("product fields must be an object".into()))?;
    for (key, value) in fields {
        let valid = match key.as_str() {
            "repository" => value.as_str().is_some_and(|s| !s.trim().is_empty()),
            "description" => value.is_string(),
            "releases" => value.is_boolean() || value.is_null(),
            "local_path" => {
                value.is_null()
                    || value
                        .as_str()
                        .is_some_and(|s| std::path::Path::new(s).is_absolute() && !s.contains('\0'))
            }
            _ => {
                return Err(Error::Invalid(format!(
                    "unknown or immutable product field: {key}"
                )));
            }
        };
        if !valid {
            return Err(Error::Invalid(format!("invalid product field: {key}")));
        }
    }
    Ok(())
}

fn apply(p: &mut Value, patch: &Value) {
    for key in ["repository", "description", "local_path", "releases"] {
        if let Some(value) = patch.get(key) {
            p[key] = value.clone();
        }
    }
}

pub fn update(s: &AppState, id: &str, patch: Value) -> Result<Value, Error> {
    check_product_id("id", id)?;
    validate(&patch)?;
    s.store.update("products", id, |p| {
        apply(p, &patch);
        p["updated_at"] = json!(format_z(s.clock.now()));
        Ok(())
    })
}

pub fn archive(s: &AppState, id: &str) -> Result<Value, Error> {
    check_product_id("id", id)?;
    s.store.update("products", id, |p| {
        if p["archived"] != true {
            let now = json!(format_z(s.clock.now()));
            p["archived"] = json!(true);
            if !p["archived_at"].is_string() {
                p["archived_at"] = now.clone();
            }
            p["updated_at"] = now;
        }
        Ok(())
    })
}
#[must_use]
pub fn summary(p: &Value) -> Value {
    let keys = [
        "id",
        "repository",
        "description",
        "local_path",
        "releases",
        "archived",
        "archived_at",
        "created_at",
        "updated_at",
    ];
    Value::Object(
        keys.into_iter()
            .filter_map(|k| p.get(k).map(|v| (k.into(), v.clone())))
            .collect(),
    )
}
