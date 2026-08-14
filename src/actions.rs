use serde::Deserialize;

use crate::error::Error;
use crate::status::Status;

const SHIPPED_TABLE: &str = include_str!("../config/actions.json");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionEffect {
    pub to: Status,
    pub next_action: Option<String>,
    pub bump: Option<String>,
    pub set_release: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionEntry {
    pub action: String,
    pub from: Status,
    pub to: Status,
    pub next_action: Option<String>,
    pub needs_bump: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ActionEntryJson {
    action: String,
    from: String,
    to: String,
    next_action: Option<String>,
    #[serde(default)]
    needs_bump: bool,
}

#[derive(Debug, Clone)]
pub struct ActionTable {
    pub entries: Vec<ActionEntry>,
}

impl Default for ActionTable {
    fn default() -> Self {
        Self::from_json(SHIPPED_TABLE.as_bytes()).expect("shipped config/actions.json")
    }
}

impl ActionTable {
    /// Load a table from JSON bytes (the same schema as `config/actions.json`).
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        let raw: Vec<ActionEntryJson> = serde_json::from_slice(bytes)?;
        if raw.is_empty() {
            return Err(Error::Invalid("action table is empty".into()));
        }
        let mut entries = Vec::with_capacity(raw.len());
        for item in raw {
            entries.push(ActionEntry {
                action: item.action,
                from: Status::parse(&item.from)?,
                to: Status::parse(&item.to)?,
                next_action: item.next_action,
                needs_bump: item.needs_bump,
            });
        }
        Ok(Self { entries })
    }

    /// # Errors
    ///
    /// Returns `Error` when the file cannot be read or is not a valid table.
    pub fn load_path(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let bytes = std::fs::read(path.as_ref())?;
        Self::from_json(&bytes)
    }

    #[must_use]
    pub fn shipped_json() -> &'static str {
        SHIPPED_TABLE
    }

    #[must_use]
    pub fn available_actions(&self, status: Status) -> Vec<String> {
        let mut names = Vec::new();
        for entry in &self.entries {
            if entry.from == status && !names.iter().any(|name| name == &entry.action) {
                names.push(entry.action.clone());
            }
        }
        names
    }

    /// Translate an action through the same loaded table production uses.
    pub fn translate(
        &self,
        status: Status,
        action: &str,
        bump: Option<&str>,
    ) -> Result<ActionEffect, Error> {
        let Some(entry) = self.entries.iter().find(|entry| entry.action == action) else {
            return Err(Error::UnknownAction);
        };
        if entry.from != status {
            return Err(Error::ActionNotAllowed);
        }
        let bump = if entry.needs_bump {
            match bump {
                Some(value @ ("patch" | "minor" | "major")) => Some((*value).to_owned()),
                _ => {
                    return Err(Error::Invalid("bump must be patch, minor, or major".into()));
                }
            }
        } else {
            None
        };
        Ok(ActionEffect {
            to: entry.to,
            next_action: entry.next_action.clone(),
            bump,
            set_release: entry.needs_bump,
        })
    }
}
