//! The haystack: one appended row per run.
//!
//! A worker's agent run, a rescue's five lines, a librarian's watermark — each
//! is one row in `runs`, appended and never edited. There is no index to
//! maintain and no review to pass: a librarian reads forward from a watermark
//! (`GET /api/runs?since=`) and summarises elsewhere. The only later write is
//! the retention sweep, which blanks the two output tails of old rows and keeps
//! every other field.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::clock::format_z;
use crate::db::Db;
use crate::error::Error;
use crate::task::Check;

/// Who appended the row. Each source has its own required fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// A worker reporting an agent run: `task_id`, `claim_id`, `outcome`.
    #[serde(rename = "worker")]
    Worker,
    /// A rescue leaving its note on a task: `task_id`, `note`.
    #[serde(rename = "rescue")]
    Rescue,
    /// A librarian recording where it read up to: `note`.
    #[serde(rename = "librarian")]
    Librarian,
}

impl Source {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Rescue => "rescue",
            Self::Librarian => "librarian",
        }
    }

    fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "worker" => Ok(Self::Worker),
            "rescue" => Ok(Self::Rescue),
            "librarian" => Ok(Self::Librarian),
            other => Err(Error::Invalid(format!("invalid run source: {other}"))),
        }
    }
}

/// Lines added, removed, files touched — whatever the worker measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStat {
    pub files: i64,
    pub insertions: i64,
    pub deletions: i64,
}

/// What a caller appends. Every field but `source` is optional at the wire;
/// [`NewRun::validate`] says which ones a source has to fill.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct NewRun {
    pub source: Option<Source>,
    #[serde(default)]
    pub worker: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub claim_id: Option<String>,
    #[serde(default)]
    pub attempt: Option<i64>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub skill_sha: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub diff_stat: Option<DiffStat>,
    #[serde(default)]
    pub agent_exit: Option<i64>,
    #[serde(default)]
    pub agent_secs: Option<i64>,
    #[serde(default)]
    pub stdout_tail: Option<String>,
    #[serde(default)]
    pub stderr_tail: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// The longest tail (or note) a row keeps, in bytes. Longer input is cut on a
/// character boundary and the row says so with `truncated`.
pub const TAIL_LIMIT_BYTES: usize = 8 * 1024;

fn non_blank(value: Option<&str>) -> bool {
    value.is_some_and(|text| !text.trim().is_empty())
}

impl NewRun {
    /// The source, or the refusal that it is missing.
    ///
    /// # Errors
    /// `Error::Invalid` when `source` is absent.
    pub fn source(&self) -> Result<Source, Error> {
        self.source
            .ok_or_else(|| Error::Invalid("source is required".into()))
    }

    /// Check the fields a source has to fill.
    ///
    /// # Errors
    /// `Error::Invalid` naming the first missing field.
    pub fn validate(&self) -> Result<Source, Error> {
        let source = self.source()?;
        let required: &[(&str, bool)] = match source {
            Source::Worker => &[
                ("task_id", non_blank(self.task_id.as_deref())),
                ("claim_id", non_blank(self.claim_id.as_deref())),
                ("outcome", non_blank(self.outcome.as_deref())),
            ],
            Source::Rescue => &[
                ("task_id", non_blank(self.task_id.as_deref())),
                ("note", non_blank(self.note.as_deref())),
            ],
            Source::Librarian => &[("note", non_blank(self.note.as_deref()))],
        };
        if let Some((field, _)) = required.iter().find(|(_, present)| !present) {
            return Err(Error::Invalid(format!(
                "{field} is required for a {} run",
                source.as_str()
            )));
        }
        Ok(source)
    }
}

/// One stored row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Run {
    pub id: i64,
    pub at: String,
    pub source: Source,
    pub worker: Option<String>,
    pub task_id: Option<String>,
    pub kind: Option<String>,
    pub claim_id: Option<String>,
    pub attempt: Option<i64>,
    pub profile: Option<String>,
    pub model: Option<String>,
    pub skill_sha: Option<String>,
    pub outcome: Option<String>,
    pub checks: Vec<Check>,
    pub commit_sha: Option<String>,
    pub diff_stat: Option<DiffStat>,
    pub agent_exit: Option<i64>,
    pub agent_secs: Option<i64>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub note: Option<String>,
    pub truncated: bool,
}

/// Cut `text` to [`TAIL_LIMIT_BYTES`] on a character boundary. Returns whether
/// anything was cut.
#[must_use]
pub fn truncate_tail(text: &str) -> (String, bool) {
    if text.len() <= TAIL_LIMIT_BYTES {
        return (text.to_owned(), false);
    }
    let mut end = TAIL_LIMIT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

/// A page of the haystack in `id` order. `next` is the `since` for the page
/// after this one, absent when this page was the last.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page {
    pub runs: Vec<Run>,
    pub next: Option<i64>,
}

pub const DEFAULT_PAGE: usize = 100;
pub const MAX_PAGE: usize = 500;

/// Append one run. The idempotency key is `(claim_id, attempt, source)`: a
/// resend of the same key returns the row already there and `false`.
///
/// # Errors
/// `Error::Invalid` when a required field of the source is missing, `Error::Db`
/// when the write fails.
pub fn append(db: &Db, new: &NewRun, now: OffsetDateTime) -> Result<(Run, bool), Error> {
    let source = new.validate()?;
    let stamp = format_z(now);
    db.with_tx(|tx| {
        if let (Some(claim_id), Some(attempt)) = (new.claim_id.as_deref(), new.attempt) {
            let existing = tx
                .query_row(
                    "SELECT id FROM runs WHERE claim_id = ?1 AND attempt = ?2 AND source = ?3",
                    params![claim_id, attempt, source.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if let Some(id) = existing {
                return Ok((read(tx, id)?, false));
            }
        }
        let mut truncated = false;
        let mut cut = |text: &Option<String>| -> Option<String> {
            text.as_deref().map(|text| {
                let (kept, was_cut) = truncate_tail(text);
                truncated |= was_cut;
                kept
            })
        };
        let stdout_tail = cut(&new.stdout_tail);
        let stderr_tail = cut(&new.stderr_tail);
        let note = cut(&new.note);
        let checks = serde_json::to_string(&new.checks)
            .map_err(|error| Error::Invalid(error.to_string()))?;
        let diff_stat = new
            .diff_stat
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| Error::Invalid(error.to_string()))?;
        tx.execute(
            "INSERT INTO runs (at, source, worker, task_id, kind, claim_id, attempt, profile,
                               model, skill_sha, outcome, checks, commit_sha, diff_stat,
                               agent_exit, agent_secs, stdout_tail, stderr_tail, note, truncated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                     ?18, ?19, ?20)",
            params![
                stamp,
                source.as_str(),
                new.worker,
                new.task_id,
                new.kind,
                new.claim_id,
                new.attempt,
                new.profile,
                new.model,
                new.skill_sha,
                new.outcome,
                checks,
                new.commit_sha,
                diff_stat,
                new.agent_exit,
                new.agent_secs,
                stdout_tail,
                stderr_tail,
                note,
                i64::from(truncated),
            ],
        )?;
        let id = tx.last_insert_rowid();
        Ok((read(tx, id)?, true))
    })
}

const COLUMNS: &str = "id, at, source, worker, task_id, kind, claim_id, attempt, profile, model,
                       skill_sha, outcome, checks, commit_sha, diff_stat, agent_exit, agent_secs,
                       stdout_tail, stderr_tail, note, truncated";

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    let source: String = row.get(2)?;
    let checks: Option<String> = row.get(12)?;
    let diff_stat: Option<String> = row.get(14)?;
    Ok(Run {
        id: row.get(0)?,
        at: row.get(1)?,
        source: Source::parse(&source).unwrap_or(Source::Worker),
        worker: row.get(3)?,
        task_id: row.get(4)?,
        kind: row.get(5)?,
        claim_id: row.get(6)?,
        attempt: row.get(7)?,
        profile: row.get(8)?,
        model: row.get(9)?,
        skill_sha: row.get(10)?,
        outcome: row.get(11)?,
        checks: checks
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default(),
        commit_sha: row.get(13)?,
        diff_stat: diff_stat.and_then(|text| serde_json::from_str(&text).ok()),
        agent_exit: row.get(15)?,
        agent_secs: row.get(16)?,
        stdout_tail: row.get(17)?,
        stderr_tail: row.get(18)?,
        note: row.get(19)?,
        truncated: row.get::<_, i64>(20)? != 0,
    })
}

fn read(conn: &Connection, id: i64) -> Result<Run, Error> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM runs WHERE id = ?1"),
        params![id],
        row_to_run,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Rows with `id > since`, ascending, at most `limit` (clamped to
/// `1..=MAX_PAGE`), optionally only one task's. `next` is set when the page is
/// full, so a reader loops `since = next` until it is absent.
///
/// # Errors
/// `Error::Db` when the query fails.
pub fn list(
    db: &Db,
    since: i64,
    limit: Option<usize>,
    task_id: Option<&str>,
) -> Result<Page, Error> {
    let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    db.with_conn(|conn| {
        let sql = format!(
            "SELECT {COLUMNS} FROM runs
             WHERE id > ?1 AND (?2 IS NULL OR task_id = ?2)
             ORDER BY id ASC LIMIT ?3"
        );
        let mut statement = conn.prepare(&sql)?;
        let limit_param = i64::try_from(limit).unwrap_or(i64::MAX);
        let runs = statement
            .query_map(params![since, task_id, limit_param], row_to_run)?
            .collect::<Result<Vec<_>, _>>()?;
        let next = (runs.len() == limit)
            .then(|| runs.last().map(|run| run.id))
            .flatten();
        Ok(Page { runs, next })
    })
}

/// How many rows name `task_id`. The Task Card carries it as `runs_count`.
///
/// # Errors
/// `Error::Db` when the query fails.
pub fn count_for_task(db: &Db, task_id: &str) -> Result<i64, Error> {
    db.with_conn(|conn| {
        Ok(conn.query_row(
            "SELECT count(*) FROM runs WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )?)
    })
}

/// Blank the two output tails of rows older than `retention_days`. Every other
/// field stays. Returns how many rows changed.
///
/// # Errors
/// `Error::Db` when the write fails.
pub fn prune_tails(db: &Db, now: OffsetDateTime, retention_days: u64) -> Result<usize, Error> {
    let days = i64::try_from(retention_days).unwrap_or(i64::MAX);
    let cutoff = format_z(now - time::Duration::days(days));
    db.with_tx(|tx| {
        Ok(tx.execute(
            "UPDATE runs SET stdout_tail = NULL, stderr_tail = NULL
             WHERE at < ?1 AND (stdout_tail IS NOT NULL OR stderr_tail IS NOT NULL)",
            params![cutoff],
        )?)
    })
}

/// The wire shape of a `Run` for tests and callers that want a `Value`.
#[must_use]
pub fn to_value(run: &Run) -> Value {
    serde_json::to_value(run).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    fn now() -> OffsetDateTime {
        datetime!(2026-09-03 12:00:00 UTC)
    }

    fn worker_run(claim_id: &str, attempt: i64) -> NewRun {
        NewRun {
            source: Some(Source::Worker),
            worker: Some("sandbox-01".into()),
            task_id: Some("t-1".into()),
            kind: Some("normal".into()),
            claim_id: Some(claim_id.into()),
            attempt: Some(attempt),
            profile: Some("fable".into()),
            outcome: Some("done".into()),
            checks: vec![Check {
                name: "cargo test --locked".into(),
                exit_code: 0,
            }],
            ..NewRun::default()
        }
    }

    /// Appending is idempotent on `(claim_id, attempt, source)`: the resend gets
    /// the same id and writes nothing. A different attempt is a new row.
    #[test]
    fn a_resend_of_the_same_key_returns_the_existing_row() {
        let db = Db::open_in_memory().unwrap();
        let (first, created) = append(&db, &worker_run("c-1", 1), now()).unwrap();
        assert!(created);
        assert_eq!(first.id, 1);
        assert_eq!(first.at, "2026-09-03T12:00:00Z");
        assert_eq!(first.checks.len(), 1);

        let mut resend = worker_run("c-1", 1);
        resend.outcome = Some("something else".into());
        let (again, created) = append(&db, &resend, now()).unwrap();
        assert!(!created);
        assert_eq!(again, first, "the resend changes nothing");

        let (second, created) = append(&db, &worker_run("c-1", 2), now()).unwrap();
        assert!(created);
        assert_eq!(second.id, 2);
        assert_eq!(count_for_task(&db, "t-1").unwrap(), 2);
    }

    /// Each source has its own required fields; the refusal names the first
    /// one missing.
    #[test]
    fn each_source_requires_its_own_fields() {
        let db = Db::open_in_memory().unwrap();
        let missing_source = NewRun::default();
        assert!(
            matches!(append(&db, &missing_source, now()), Err(Error::Invalid(message)) if message.contains("source"))
        );

        let mut worker = worker_run("c-1", 1);
        worker.outcome = Some("   ".into());
        assert!(
            matches!(append(&db, &worker, now()), Err(Error::Invalid(message)) if message.contains("outcome"))
        );

        let rescue = NewRun {
            source: Some(Source::Rescue),
            task_id: Some("t-1".into()),
            ..NewRun::default()
        };
        assert!(
            matches!(append(&db, &rescue, now()), Err(Error::Invalid(message)) if message.contains("note"))
        );
        let rescue = NewRun {
            note: Some("five lines".into()),
            ..rescue
        };
        assert!(append(&db, &rescue, now()).unwrap().1);

        let librarian = NewRun {
            source: Some(Source::Librarian),
            ..NewRun::default()
        };
        assert!(
            matches!(append(&db, &librarian, now()), Err(Error::Invalid(message)) if message.contains("note"))
        );
        let librarian = NewRun {
            note: Some("watermark 42".into()),
            ..librarian
        };
        let (row, _) = append(&db, &librarian, now()).unwrap();
        assert_eq!(row.source, Source::Librarian);
        assert!(
            row.claim_id.is_none(),
            "a librarian has no claim, so no idempotency key"
        );
        // Without a key every append is a new row.
        let (row2, created) = append(&db, &librarian, now()).unwrap();
        assert!(created);
        assert_ne!(row.id, row2.id);
    }

    /// Tails over the limit are cut on a char boundary and the row says so.
    #[test]
    fn long_tails_are_cut_and_marked() {
        let db = Db::open_in_memory().unwrap();
        let mut run = worker_run("c-1", 1);
        // Multi-byte characters straddle the limit.
        run.stdout_tail = Some("あ".repeat(TAIL_LIMIT_BYTES));
        run.stderr_tail = Some("x".repeat(10));
        let (row, _) = append(&db, &run, now()).unwrap();
        assert!(row.truncated);
        let kept = row.stdout_tail.unwrap();
        assert!(kept.len() <= TAIL_LIMIT_BYTES);
        assert!(kept.chars().all(|c| c == 'あ'));
        assert_eq!(row.stderr_tail.as_deref(), Some("xxxxxxxxxx"));

        let (short, _) = append(&db, &worker_run("c-2", 1), now()).unwrap();
        assert!(!short.truncated);
        assert_eq!(truncate_tail("abc"), ("abc".to_owned(), false));
    }

    /// `since` pages forward in id order; `next` is the cursor while a page is
    /// full and absent on the last one. `task_id` narrows without breaking the
    /// cursor.
    #[test]
    fn listing_pages_forward_by_id_with_a_cursor() {
        let db = Db::open_in_memory().unwrap();
        for attempt in 1..=5 {
            append(&db, &worker_run("c-1", attempt), now()).unwrap();
        }
        let mut other = worker_run("c-9", 1);
        other.task_id = Some("t-2".into());
        append(&db, &other, now()).unwrap();

        let page = list(&db, 0, Some(2), None).unwrap();
        assert_eq!(
            page.runs.iter().map(|run| run.id).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(page.next, Some(2));
        let page = list(&db, 2, Some(2), None).unwrap();
        assert_eq!(
            page.runs.iter().map(|run| run.id).collect::<Vec<_>>(),
            [3, 4]
        );
        assert_eq!(page.next, Some(4));
        let page = list(&db, 4, Some(2), None).unwrap();
        assert_eq!(
            page.runs.iter().map(|run| run.id).collect::<Vec<_>>(),
            [5, 6]
        );
        assert_eq!(page.next, Some(6), "a full page always offers a cursor");
        let page = list(&db, 6, Some(2), None).unwrap();
        assert!(page.runs.is_empty());
        assert_eq!(page.next, None);

        let only_t2 = list(&db, 0, None, Some("t-2")).unwrap();
        assert_eq!(only_t2.runs.len(), 1);
        assert_eq!(only_t2.runs[0].task_id.as_deref(), Some("t-2"));
        assert_eq!(only_t2.next, None);

        // The limit is clamped, never trusted.
        assert_eq!(list(&db, 0, Some(0), None).unwrap().runs.len(), 1);
        assert_eq!(list(&db, 0, Some(10_000), None).unwrap().runs.len(), 6);
    }

    /// The sweep blanks only the tails, only past retention, and keeps the rest.
    #[test]
    fn the_sweep_blanks_old_tails_and_nothing_else() {
        let db = Db::open_in_memory().unwrap();
        let mut old = worker_run("c-old", 1);
        old.stdout_tail = Some("old out".into());
        old.stderr_tail = Some("old err".into());
        append(&db, &old, now() - time::Duration::days(91)).unwrap();
        let mut fresh = worker_run("c-new", 1);
        fresh.stdout_tail = Some("new out".into());
        append(&db, &fresh, now() - time::Duration::days(89)).unwrap();

        assert_eq!(prune_tails(&db, now(), 90).unwrap(), 1);
        let page = list(&db, 0, None, None).unwrap();
        let old_row = &page.runs[0];
        assert_eq!(old_row.stdout_tail, None);
        assert_eq!(old_row.stderr_tail, None);
        assert_eq!(
            old_row.outcome.as_deref(),
            Some("done"),
            "other fields stay"
        );
        assert_eq!(old_row.checks.len(), 1);
        assert_eq!(page.runs[1].stdout_tail.as_deref(), Some("new out"));
        // Running it again changes nothing.
        assert_eq!(prune_tails(&db, now(), 90).unwrap(), 0);
    }
}
