use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationIntent {
    pub task_id: String,
    pub kind: String,
    pub commit_sha: String,
    pub claim_id: String,
    pub created_at: String,
    pub state: String,
}

/// # Errors
///
/// Returns `Error` when the outbox directory cannot be created or written.
pub fn enqueue(outbox_dir: &Path, intent: &NotificationIntent) -> Result<(), Error> {
    fs::create_dir_all(outbox_dir)?;
    let file = format!(
        "{}-{}.json",
        intent.created_at.replace([':', '+'], ""),
        intent.task_id
    );
    let path = outbox_dir.join(file);
    fs::write(path, serde_json::to_vec_pretty(intent)?)?;
    Ok(())
}

/// # Errors
///
/// Returns `Error` when the outbox cannot be read or a file is not JSON.
pub fn list_pending(outbox_dir: &Path) -> Result<Vec<NotificationIntent>, Error> {
    if !outbox_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut intents = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(outbox_dir)?.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read(&path)?;
        let intent: NotificationIntent = serde_json::from_slice(&raw)?;
        if intent.state == "pending" {
            intents.push(intent);
        }
    }
    Ok(intents)
}
