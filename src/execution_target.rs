//! Execution target definitions live in one ledger record. An operator file
//! seeds the record once; afterwards HTTP and MCP edit it under the writer lock.
use crate::{
    AppState, Error,
    ledger::{Store, StoreAccess},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

const COLLECTION: &str = "settings";
const ID: &str = "execution_targets";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTargets {
    pub labels: Vec<String>,
    pub default: Option<String>,
}

/// File-name-safe names only: `^[a-z0-9][a-z0-9_-]*$`.
pub fn check_label(label: &str) -> Result<(), Error> {
    let safe = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let mut bytes = label.bytes();
    if bytes.next().is_some_and(safe) && bytes.all(|b| safe(b) || b == b'-' || b == b'_') {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "execution target label must match ^[a-z0-9][a-z0-9_-]*$: {label:?}"
        )))
    }
}

impl ExecutionTargets {
    fn check(&self) -> Result<(), Error> {
        let mut names = HashSet::new();
        for label in &self.labels {
            check_label(label)?;
            if !names.insert(label) {
                return Err(Error::Invalid(format!(
                    "duplicate execution target label: {label}"
                )));
            }
        }
        if self.default.as_ref().is_some_and(|d| !names.contains(d)) {
            return Err(Error::Invalid(
                "default execution target must be in labels".into(),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn contains(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }
}

pub fn load_file(path: &str) -> Result<ExecutionTargets, Error> {
    let invalid = |message| Error::Invalid(format!("EXECUTION_TARGETS_FILE: {message}"));
    let text = std::fs::read_to_string(path).map_err(|e| invalid(format!("{path}: {e}")))?;
    let value: Value = serde_norway::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    let targets: ExecutionTargets =
        serde_json::from_value(value).map_err(|e| invalid(e.to_string()))?;
    targets.check().map_err(|e| invalid(e.to_string()))?;
    Ok(targets)
}

/// Current definitions; no record means none are defined yet.
pub fn read(a: &StoreAccess<'_>) -> Result<ExecutionTargets, Error> {
    let record = match a.get(COLLECTION, ID) {
        Ok(record) => record,
        Err(Error::NotFound(_)) => return Ok(ExecutionTargets::default()),
        Err(error) => return Err(error),
    };
    let targets: ExecutionTargets =
        serde_json::from_value(json!({"labels":record["labels"],"default":record["default"]}))
            .map_err(|e| Error::Invalid(format!("{COLLECTION}/{ID}: {e}")))?;
    targets.check()?;
    Ok(targets)
}

pub fn get(s: &AppState) -> Result<ExecutionTargets, Error> {
    s.store.transaction(read)
}

/// Seed the ledger from the operator file only while it has no definitions.
pub fn import(store: &Store, path: &str) -> Result<(), Error> {
    store.transaction(|a| match a.get(COLLECTION, ID) {
        Ok(_) => Ok(()),
        Err(Error::NotFound(_)) => a.create(COLLECTION, ID, json!(load_file(path)?)).map(drop),
        Err(error) => Err(error),
    })
}

fn edit(
    s: &AppState,
    operation: impl FnOnce(&mut ExecutionTargets) -> Result<(), Error>,
) -> Result<ExecutionTargets, Error> {
    s.store.transaction(|a| {
        let mut targets = read(a)?;
        operation(&mut targets)?;
        targets.check()?;
        let mut record = match a.get(COLLECTION, ID) {
            Ok(record) => record,
            Err(Error::NotFound(_)) => json!({}),
            Err(error) => return Err(error),
        };
        record["labels"] = json!(targets.labels);
        record["default"] = json!(targets.default);
        a.put(COLLECTION, ID, record)?;
        Ok(targets)
    })
}

pub fn create(s: &AppState, label: &str) -> Result<ExecutionTargets, Error> {
    check_label(label)?;
    edit(s, |targets| {
        if targets.contains(label) {
            return Err(Error::Conflict(format!(
                "execution target already exists: {label}"
            )));
        }
        targets.labels.push(label.into());
        Ok(())
    })
}

/// Existing task references stay; only new assignments and claims stop.
pub fn delete(s: &AppState, label: &str) -> Result<ExecutionTargets, Error> {
    check_label(label)?;
    edit(s, |targets| {
        if !targets.contains(label) {
            return Err(Error::NotFound(format!(
                "execution target not found: {label}"
            )));
        }
        if targets.default.as_deref() == Some(label) {
            return Err(Error::Conflict(
                "the default execution target cannot be deleted".into(),
            ));
        }
        targets.labels.retain(|l| l != label);
        Ok(())
    })
}
