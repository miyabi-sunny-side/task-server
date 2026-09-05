#![allow(clippy::needless_pass_by_value)]
use crate::{AppState, Error, format_z};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Product {
    pub id: String,
    pub repository: String,
    pub description: String,
    pub releases: bool,
    #[serde(default)]
    pub archived: bool,
}
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
pub fn put(s: &AppState, id: &str, v: Value) -> Result<Value, Error> {
    check_product_id("id", id)?;
    if v["repository"].as_str().is_none_or(|r| r.trim().is_empty()) {
        return Err(Error::Invalid("repository is required".into()));
    }
    s.store.transaction(|a| {
        let mut p = match a.get("products", id) {
            Ok(v) => v,
            Err(Error::NotFound(_)) => {
                json!({"id":id,"created_at":format_z(s.clock.now()),"archived":false})
            }
            Err(e) => return Err(e),
        };
        for (k, default) in [
            ("repository", json!("")),
            ("description", json!("")),
            ("releases", json!(false)),
        ] {
            p[k] = v.get(k).cloned().unwrap_or(default);
        }
        p["updated_at"] = json!(format_z(s.clock.now()));
        a.put("products", id, p)
    })
}
pub fn rescan(s: &AppState) -> Result<Value, Error> {
    let root = s
        .projects_dir
        .as_ref()
        .ok_or_else(|| Error::Conflict("APP_PROJECTS_DIR is not configured".into()))?;
    let report = crate::scan::scan(root)?;
    s.store.transaction(|a| {
        let existing = a.list("products")?;
        let report = report.with_previous_releases(|id| existing.iter().find(|p| p["id"] == id).and_then(|p| p["releases"].as_bool()));
        let ids = report.products.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        for scanned in report.products {
            let mut record = existing.iter().find(|p| p["id"] == scanned.id).cloned().unwrap_or_else(|| json!({"id": scanned.id, "created_at": format_z(s.clock.now()), "updated_at": format_z(s.clock.now())}));
            record["repository"] = json!(scanned.repository);
            record["description"] = json!(scanned.description);
            record["releases"] = json!(scanned.releases);
            record["archived"] = json!(false);
            if record.get("archived_at").is_some() { record["archived_at"] = Value::Null; }
            a.put("products", &scanned.id, record)?;
        }
        if !ids.is_empty() {
            for mut record in existing {
                let id = record["id"].as_str().unwrap_or_default().to_owned();
                if !ids.contains(&id) {
                    record["archived"] = json!(true);
                    if record["archived_at"].is_null() { record["archived_at"] = json!(format_z(s.clock.now())); }
                    a.put("products", &id, record)?;
                }
            }
        }
        Ok(json!({"products": a.list("products")?.iter().map(summary).collect::<Vec<_>>(), "count": ids.len(), "skipped": report.skipped.len(), "skipped_archive_all": ids.is_empty()}))
    })
}
#[must_use]
pub fn summary(p: &Value) -> Value {
    let keys = [
        "id",
        "repository",
        "description",
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
