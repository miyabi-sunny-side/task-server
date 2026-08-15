use rusqlite::{Connection, Row, ToSql};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::clock::format_z;
use crate::db::Db;
use crate::error::Error;
use crate::product::{self, check_product_id};

const COLUMNS: &str = "id, title, body, status, kind, product_id, priority, branch, claimed_by, \
                       claim_id, claimed_at, claim_expires_at, commit_sha, verification, \
                       release_tag, created_at, updated_at, merge_target_task_id, checks_json";

/// Every status, in vocabulary order. Used to enumerate legal transitions.
pub(crate) const ALL_STATUSES: [TaskStatus; 9] = [
    TaskStatus::Draft,
    TaskStatus::Ready,
    TaskStatus::Wip,
    TaskStatus::Done,
    TaskStatus::Merged,
    TaskStatus::Released,
    TaskStatus::Blocked,
    TaskStatus::Cancelled,
    TaskStatus::Dropped,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Draft,
    Ready,
    Wip,
    Done,
    Merged,
    Released,
    Blocked,
    Cancelled,
    Dropped,
}

impl TaskStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Wip => "wip",
            Self::Done => "done",
            Self::Merged => "merged",
            Self::Released => "released",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::Dropped => "dropped",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "draft" => Ok(Self::Draft),
            "ready" => Ok(Self::Ready),
            "wip" => Ok(Self::Wip),
            "done" => Ok(Self::Done),
            "merged" => Ok(Self::Merged),
            "released" => Ok(Self::Released),
            "blocked" => Ok(Self::Blocked),
            "cancelled" => Ok(Self::Cancelled),
            "dropped" => Ok(Self::Dropped),
            other => Err(Error::Invalid(format!("invalid status: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskKind {
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "instant:merge")]
    InstantMerge,
}

impl TaskKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::InstantMerge => "instant:merge",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "normal" => Ok(Self::Normal),
            "instant:merge" => Ok(Self::InstantMerge),
            other => Err(Error::Invalid(format!("invalid kind: {other}"))),
        }
    }
}

/// One verification a worker ran before asking for a merge. `exit_code` is the
/// process status, so `0` is the only pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub exit_code: i64,
}

/// A product with merged work waiting for a release tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Releasable {
    pub product_id: String,
    pub task_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub body: String,
    pub status: TaskStatus,
    pub kind: TaskKind,
    pub product_id: Option<String>,
    pub priority: i64,
    pub branch: Option<String>,
    pub claimed_by: Option<String>,
    pub claim_id: Option<String>,
    pub claimed_at: Option<String>,
    pub claim_expires_at: Option<String>,
    pub commit_sha: Option<String>,
    pub verification: Option<String>,
    pub release_tag: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Set on an `instant:merge` task: the task this merge lands.
    #[serde(default)]
    pub merge_target_task_id: Option<String>,
    /// Decoded from `checks_json`; the column itself is never exposed.
    #[serde(default)]
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTask {
    pub id: String,
    pub title: String,
    pub body: String,
    pub product_id: Option<String>,
    pub kind: TaskKind,
    pub priority: i64,
}

/// The attributes a PATCH may change. A `None` field is left as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub product_id: Option<String>,
    pub priority: Option<i64>,
    pub branch: Option<String>,
}

/// Create a task in `draft`.
///
/// Registration files ordinary work only. An `instant:merge` task is issued by
/// [`issue_merge`] against a target it can name, and a hand-made one would be a
/// merge with no target: claimed ahead of every other task, impossible to
/// report, and so a standing block on the queue. The refusal lives here rather
/// than in a transport, so HTTP and MCP cannot drift apart on it.
pub fn create(db: &Db, new: &NewTask, now: OffsetDateTime) -> Result<Task, Error> {
    if new.kind == TaskKind::InstantMerge {
        return Err(Error::Invalid(format!(
            "an {} task is issued by the control plane (POST /api/merges) against the task it \
             lands, and cannot be created directly",
            TaskKind::InstantMerge.as_str()
        )));
    }
    if new.id.trim().is_empty() {
        return Err(Error::Invalid("id is required".into()));
    }
    if new.title.trim().is_empty() {
        return Err(Error::Invalid("title is required".into()));
    }
    if let Some(product_id) = &new.product_id {
        check_product_id("product_id", product_id)?;
    }
    let stamp = format_z(now);
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO tasks (id, title, body, status, kind, product_id, priority, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            rusqlite::params![
                new.id,
                new.title,
                new.body,
                TaskStatus::Draft.as_str(),
                new.kind.as_str(),
                new.product_id,
                new.priority,
                stamp,
            ],
        )?;
        read(conn, &new.id)
    })
}

pub fn get(db: &Db, id: &str) -> Result<Task, Error> {
    db.with_conn(|conn| read(conn, id))
}

/// All tasks, oldest first.
pub fn list(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!("SELECT {COLUMNS} FROM tasks ORDER BY created_at ASC, id ASC"),
            &[],
        )
    })
}

/// Everything that is not `released`, oldest first. The default listing.
pub fn list_active(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks WHERE status != ?1 ORDER BY created_at ASC, id ASC"
            ),
            &[&TaskStatus::Released.as_str()],
        )
    })
}

pub fn list_by_status(db: &Db, status: TaskStatus) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks WHERE status = ?1 ORDER BY created_at ASC, id ASC"
            ),
            &[&status.as_str()],
        )
    })
}

/// Apply `patch` to an existing task. Only the attributes the patch carries
/// change; status and lease columns are owned by the workflow, not by PATCH.
pub fn update(db: &Db, id: &str, patch: &TaskPatch, now: OffsetDateTime) -> Result<Task, Error> {
    if let Some(title) = &patch.title
        && title.trim().is_empty()
    {
        return Err(Error::Invalid("title is required".into()));
    }
    if let Some(product_id) = &patch.product_id {
        check_product_id("product_id", product_id)?;
    }
    let stamp = format_z(now);
    db.with_tx(|tx| {
        let task = read(tx, id)?;
        tx.execute(
            "UPDATE tasks SET title = ?2, body = ?3, product_id = ?4, priority = ?5, branch = ?6,
                    updated_at = ?7
             WHERE id = ?1",
            rusqlite::params![
                id,
                patch.title.as_deref().unwrap_or(&task.title),
                patch.body.as_deref().unwrap_or(&task.body),
                patch.product_id.as_deref().or(task.product_id.as_deref()),
                patch.priority.unwrap_or(task.priority),
                patch.branch.as_deref().or(task.branch.as_deref()),
                stamp,
            ],
        )?;
        read(tx, id)
    })
}

/// The statuses a human may actually press on `task`.
///
/// `merged` and `released` are deliberately absent: a task lands only when a
/// merge reported green checks, and it ships only through a product release.
/// The transition table itself still allows both, because the control plane
/// goes through it.
#[must_use]
pub fn available_transitions(task: &Task) -> Vec<TaskStatus> {
    ALL_STATUSES
        .into_iter()
        .filter(|to| !matches!(to, TaskStatus::Merged | TaskStatus::Released))
        .filter(|&to| can_transition(task.status, to))
        .collect()
}

/// Whether the owning product ships releases. A task without a product does not.
fn product_releases(conn: &Connection, product_id: Option<&str>) -> Result<bool, Error> {
    match product_id {
        Some(product_id) => Ok(product::read(conn, product_id)?.releases),
        None => Ok(false),
    }
}

/// The catalogue is the register of product identity, and `ready` is where it
/// is checked. Registering a task for an unknown product is allowed on purpose:
/// whoever files the work does not have to curate the catalogue first. What is
/// not allowed is handing that task to a worker, because nobody could say which
/// repository it belongs to.
///
/// The check fires on the transition only. A row that reached `ready` or beyond
/// before its product left the catalogue keeps its status; nothing is demoted.
fn check_catalogued(conn: &Connection, task: &Task) -> Result<(), Error> {
    let Some(product_id) = task.product_id.as_deref() else {
        return Err(Error::Precondition {
            code: "product_required",
            message: format!(
                "task {} has no product_id, so it cannot become ready; \
                 set one that is in the product catalogue",
                task.id
            ),
        });
    };
    match product::read(conn, product_id) {
        Ok(_) => Ok(()),
        Err(Error::NotFound) => Err(Error::Precondition {
            code: "product_not_catalogued",
            message: format!(
                "product '{product_id}' is not in the product catalogue, \
                 so task {} cannot become ready; \
                 add it first with PUT /api/products/{product_id}",
                task.id
            ),
        }),
        Err(other) => Err(other),
    }
}

/// Move a task to `to`, refusing transitions the table forbids, promotions of
/// work whose product is not catalogued, and releases the owning product does
/// not want.
pub fn set_status(db: &Db, id: &str, to: TaskStatus, now: OffsetDateTime) -> Result<Task, Error> {
    let stamp = format_z(now);
    db.with_tx(|tx| {
        let task = read(tx, id)?;
        if !can_transition(task.status, to) {
            return Err(Error::Invalid(format!(
                "cannot move task {id} from {} to {}",
                task.status.as_str(),
                to.as_str()
            )));
        }
        if to == TaskStatus::Ready {
            check_catalogued(tx, &task)?;
        }
        if to == TaskStatus::Released && !product_releases(tx, task.product_id.as_deref())? {
            let product_id = task.product_id.as_deref().unwrap_or("<none>");
            return Err(Error::Invalid(format!(
                "product {product_id} does not release"
            )));
        }
        tx.execute(
            "UPDATE tasks SET status = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, to.as_str(), stamp],
        )?;
        read(tx, id)
    })
}

/// Move a task the way a human or an administrative surface moves it.
///
/// `merged` and `released` are earned, not pressed: a task lands only when a
/// merge reported green checks, and it ships only through a product release.
/// Every operator surface — the HTTP status route and the MCP `task_set_status`
/// tool — goes through here, so neither can become a way around the other. The
/// control plane itself keeps calling [`set_status`] directly, because that is
/// how it grants both.
pub fn set_status_by_operator(
    db: &Db,
    id: &str,
    to: TaskStatus,
    now: OffsetDateTime,
) -> Result<Task, Error> {
    if matches!(to, TaskStatus::Merged | TaskStatus::Released) {
        return Err(Error::Invalid(format!(
            "{} is granted by the control plane (POST /api/merges, POST /api/releases), \
             not by a status change",
            to.as_str()
        )));
    }
    set_status(db, id, to, now)
}

/// The rows a claim may take: anything still `ready`, plus a `wip` task whose
/// lease has run out, so a worker that died does not strand its task forever.
///
/// `{now}` stands in for the placeholder carrying the current time; the caller
/// substitutes the index it bound. Timestamps are written by [`format_z`] as
/// fixed-width `YYYY-MM-DDTHH:MM:SSZ` in UTC, so a lexicographic `<=` is a
/// chronological `<=` and sqlite needs no date parsing here.
const CLAIMABLE: &str = "(status = 'ready'
                          OR (status = 'wip' AND claim_expires_at IS NOT NULL
                              AND claim_expires_at <= {now}))";

/// Hand the next claimable task to `worker`. The row is only taken while it is
/// still claimable, so no two live leases ever cover the same task. Taking over
/// an expired lease issues a new `claim_id`, which is what invalidates the
/// abandoned one: its holder's report becomes an [`Error::ClaimMismatch`].
pub fn claim(
    db: &Db,
    worker: &str,
    now: OffsetDateTime,
    ttl_secs: u64,
) -> Result<Option<Task>, Error> {
    if worker.trim().is_empty() {
        return Err(Error::Invalid("worker is required".into()));
    }
    let ttl = i64::try_from(ttl_secs).unwrap_or(i64::MAX);
    let claimed_at = format_z(now);
    let claim_expires_at = format_z(now + time::Duration::seconds(ttl));
    let select_sql = format!(
        "SELECT {COLUMNS} FROM tasks WHERE {}
         ORDER BY CASE kind WHEN 'instant:merge' THEN 0 ELSE 1 END,
                  priority DESC, created_at ASC, id ASC
         LIMIT 1",
        CLAIMABLE.replace("{now}", "?1")
    );
    // The guard repeats the candidate predicate exactly; a narrower one would
    // leave an expired lease forever selected and never taken, spinning the loop.
    let update_sql = format!(
        "UPDATE tasks SET status = 'wip', claimed_by = ?2, claim_id = ?3, claimed_at = ?4,
                claim_expires_at = ?5, updated_at = ?4
         WHERE id = ?1 AND {}",
        CLAIMABLE.replace("{now}", "?4")
    );
    db.with_tx(|tx| {
        loop {
            let Some(task) = query_all(tx, &select_sql, &[&claimed_at])?.pop() else {
                return Ok(None);
            };
            let claim_id = uuid::Uuid::new_v4().to_string();
            let updated = tx.execute(
                &update_sql,
                rusqlite::params![task.id, worker, claim_id, claimed_at, claim_expires_at],
            )?;
            if updated > 0 {
                // One task, one branch: a claim without a branch gets the name
                // derived from the task id. An explicit branch is never rewritten.
                tx.execute(
                    "UPDATE tasks SET branch = ?2 WHERE id = ?1 AND branch IS NULL",
                    rusqlite::params![task.id, format!("task/{}", task.id)],
                )?;
                return read(tx, &task.id).map(Some);
            }
        }
    })
}

/// Accept a worker's result for the lease `claim_id`.
///
/// For an `instant:merge` task this is the gate onto the main line: the report
/// is only accepted when every check passed, and accepting it lands the target
/// task in the same transaction. A refused report leaves both rows untouched.
pub fn report(
    db: &Db,
    claim_id: &str,
    commit_sha: &str,
    verification: &str,
    checks: &[Check],
    now: OffsetDateTime,
) -> Result<Task, Error> {
    if commit_sha.trim().is_empty() || verification.trim().is_empty() {
        return Err(Error::Invalid(
            "commit_sha and verification are required".into(),
        ));
    }
    let stamp = format_z(now);
    let checks_json = if checks.is_empty() {
        None
    } else {
        Some(serde_json::to_string(checks)?)
    };
    db.with_tx(|tx| {
        let sql = format!("SELECT {COLUMNS} FROM tasks WHERE claim_id = ?1");
        let Some(task) = query_all(tx, &sql, &[&claim_id])?.pop() else {
            return Err(Error::ClaimMismatch);
        };
        // The gate belongs to the report, not to one status: a merge that
        // already landed must still be told the checks passed, or a repeat
        // without evidence would read as "the merge went through with no
        // checks" on the idempotent path.
        if task.kind == TaskKind::InstantMerge {
            check_gate(&task, checks)?;
        }
        match task.status {
            TaskStatus::Wip => {
                tx.execute(
                    "UPDATE tasks SET status = 'done', commit_sha = ?2, verification = ?3,
                            checks_json = ?4, updated_at = ?5
                     WHERE id = ?1",
                    rusqlite::params![task.id, commit_sha, verification, checks_json, stamp],
                )?;
                if task.kind == TaskKind::InstantMerge {
                    land_merge_target(tx, &task, &stamp)?;
                }
                read(tx, &task.id)
            }
            TaskStatus::Done if task.commit_sha.as_deref() == Some(commit_sha) => Ok(task),
            TaskStatus::Done => Err(Error::Invalid(format!(
                "task {} was already reported with a different commit",
                task.id
            ))),
            other => Err(Error::Invalid(format!(
                "task {} cannot be reported from {}",
                task.id,
                other.as_str()
            ))),
        }
    })
}

/// Nothing reaches the main line without evidence: a merge report must carry
/// checks, and every one of them must have exited zero. Applied to every report
/// on a merge task, landed or not, so the answer never depends on the order the
/// reports arrived in.
fn check_gate(task: &Task, checks: &[Check]) -> Result<(), Error> {
    if checks.is_empty() {
        return Err(Error::Invalid(format!(
            "merge task {} is only reported with passing checks",
            task.id
        )));
    }
    if let Some(failed) = checks.iter().find(|check| check.exit_code != 0) {
        return Err(Error::Invalid(format!(
            "merge task {} is refused: check '{}' exited {}",
            task.id, failed.name, failed.exit_code
        )));
    }
    Ok(())
}

/// Move the task a finished merge landed from `done` to `merged`.
fn land_merge_target(tx: &Connection, merge: &Task, stamp: &str) -> Result<(), Error> {
    let Some(target_id) = merge.merge_target_task_id.as_deref() else {
        return Err(Error::Invalid(format!(
            "merge task {} has no target to land",
            merge.id
        )));
    };
    let target = read(tx, target_id)?;
    if target.status != TaskStatus::Done {
        return Err(Error::Invalid(format!(
            "task {target_id} is {}, so merge task {} cannot land it",
            target.status.as_str(),
            merge.id
        )));
    }
    tx.execute(
        "UPDATE tasks SET status = 'merged', updated_at = ?2 WHERE id = ?1",
        rusqlite::params![target_id, stamp],
    )?;
    Ok(())
}

/// The tasks a human may press "merge" on: finished normal work that carries
/// the branch and commit a worker needs, and that no live merge already owns.
pub fn mergeable(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks
                 WHERE kind = 'normal' AND status = 'done'
                   AND branch IS NOT NULL AND commit_sha IS NOT NULL
                   AND NOT EXISTS (
                     SELECT 1 FROM tasks live
                     WHERE live.merge_target_task_id = tasks.id
                       AND live.status NOT IN ('cancelled', 'dropped')
                   )
                 ORDER BY created_at ASC, id ASC"
            ),
            &[],
        )
    })
}

/// Merge tasks that have been issued and not finished yet.
pub fn pending_merges(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks
                 WHERE kind = 'instant:merge'
                   AND status NOT IN ('done', 'cancelled', 'dropped')
                 ORDER BY created_at ASC, id ASC"
            ),
            &[],
        )
    })
}

/// The id of the first merge task that lands `target_id`. Derived, so a target
/// and its merge are readable as a pair.
#[must_use]
pub fn merge_task_id(target_id: &str) -> String {
    format!("merge:{target_id}")
}

/// The id of the `attempt`-th merge for `target_id`. The first attempt keeps the
/// plain derived id; a retry after a cancelled or dropped attempt appends `~2`,
/// `~3`, … `~` is unreserved in a URI, so the id stays one path segment.
fn merge_attempt_id(target_id: &str, attempt: u32) -> String {
    match attempt {
        1 => merge_task_id(target_id),
        n => format!("{}~{n}", merge_task_id(target_id)),
    }
}

/// The first merge id for `target_id` that no row has taken yet.
///
/// The partial unique index, not this walk, is what forbids a second *live*
/// merge; this only keeps a permitted retry from colliding with the primary key
/// of an attempt that was cancelled or dropped. It runs inside the caller's
/// transaction, so a racing issue serializes behind it and then loses on the
/// index instead of stealing the id.
fn free_merge_id(conn: &Connection, target_id: &str) -> Result<String, Error> {
    let mut attempt = 1;
    loop {
        let id = merge_attempt_id(target_id, attempt);
        let taken: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
            [&id],
            |row| row.get(0),
        )?;
        if !taken {
            return Ok(id);
        }
        attempt += 1;
    }
}

/// Issue the `instant:merge` task that lands `target_id`.
///
/// The merge inherits the target's product, branch and commit, so a worker
/// reads one task and knows exactly which branch to rebase onto main.
pub fn issue_merge(db: &Db, target_id: &str, now: OffsetDateTime) -> Result<Task, Error> {
    let stamp = format_z(now);
    db.with_tx(|tx| {
        let target = read(tx, target_id)?;
        if target.kind != TaskKind::Normal {
            return Err(Error::Invalid(format!(
                "task {target_id} is {}, and only normal work is merged",
                target.kind.as_str()
            )));
        }
        if target.status != TaskStatus::Done {
            return Err(Error::Invalid(format!(
                "task {target_id} is {}, so it is not ready to merge",
                target.status.as_str()
            )));
        }
        let (Some(branch), Some(commit_sha)) = (&target.branch, &target.commit_sha) else {
            return Err(Error::Invalid(format!(
                "task {target_id} has no branch and commit to merge"
            )));
        };
        let id = free_merge_id(tx, target_id)?;
        tx.execute(
            "INSERT INTO tasks (id, title, body, status, kind, product_id, priority, branch,
                                commit_sha, merge_target_task_id, created_at, updated_at)
             VALUES (?1, ?2, '', 'ready', 'instant:merge', ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            rusqlite::params![
                id,
                format!("merge {target_id}: {}", target.title),
                target.product_id,
                target.priority,
                branch,
                commit_sha,
                target_id,
                stamp,
            ],
        )
        .map_err(|err| merge_conflict(err, target_id))?;
        read(tx, &id)
    })
}

/// The partial unique index (and the primary key) is what actually forbids a
/// second live merge; a constraint violation here is a conflict, not a bug.
fn merge_conflict(err: rusqlite::Error, target_id: &str) -> Error {
    match err {
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Error::Conflict(format!("task {target_id} already has a merge in flight"))
        }
        other => other.into(),
    }
}

/// Merged work waiting for a release tag, per releasing product.
pub fn releasable(db: &Db) -> Result<Vec<Releasable>, Error> {
    db.with_conn(|conn| {
        let mut statement = conn.prepare(
            "SELECT tasks.product_id, count(*) FROM tasks
             JOIN products ON products.id = tasks.product_id
             WHERE products.releases = 1 AND tasks.kind = 'normal' AND tasks.status = 'merged'
             GROUP BY tasks.product_id
             ORDER BY tasks.product_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Releasable {
                product_id: row.get(0)?,
                task_count: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })
}

/// Stamp every merged task of `product_id` with `tag` and move it to
/// `released`. All of them or none: one transaction, one tag.
pub fn release_product(
    db: &Db,
    product_id: &str,
    tag: &str,
    now: OffsetDateTime,
) -> Result<Vec<Task>, Error> {
    if tag.trim().is_empty() {
        return Err(Error::Invalid("tag is required".into()));
    }
    let stamp = format_z(now);
    db.with_tx(|tx| {
        let product = product::read(tx, product_id)?;
        if !product.releases {
            return Err(Error::Conflict(format!(
                "product {product_id} does not release"
            )));
        }
        let sql = format!(
            "SELECT {COLUMNS} FROM tasks
             WHERE product_id = ?1 AND kind = 'normal' AND status = 'merged'
             ORDER BY created_at ASC, id ASC"
        );
        let targets = query_all(tx, &sql, &[&product_id])?;
        if targets.is_empty() {
            return Err(Error::Conflict(format!(
                "product {product_id} has no merged task to release"
            )));
        }
        tx.execute(
            "UPDATE tasks SET status = 'released', release_tag = ?2, updated_at = ?3
             WHERE product_id = ?1 AND kind = 'normal' AND status = 'merged'",
            rusqlite::params![product_id, tag, stamp],
        )?;
        targets.iter().map(|task| read(tx, &task.id)).collect()
    })
}

/// Whether `from → to` is allowed. Pure table; product quality attributes are
/// checked separately by [`set_status`].
#[must_use]
pub fn can_transition(from: TaskStatus, to: TaskStatus) -> bool {
    if matches!(
        from,
        TaskStatus::Released | TaskStatus::Dropped | TaskStatus::Cancelled
    ) {
        return false;
    }
    if matches!(
        to,
        TaskStatus::Blocked | TaskStatus::Cancelled | TaskStatus::Dropped
    ) {
        return from != to;
    }
    matches!(
        (from, to),
        (TaskStatus::Draft | TaskStatus::Blocked, TaskStatus::Ready)
            | (TaskStatus::Ready, TaskStatus::Wip)
            | (TaskStatus::Wip, TaskStatus::Done | TaskStatus::Ready)
            | (TaskStatus::Done, TaskStatus::Merged)
            | (TaskStatus::Merged, TaskStatus::Released)
    )
}

pub(crate) fn read(conn: &Connection, id: &str) -> Result<Task, Error> {
    let sql = format!("SELECT {COLUMNS} FROM tasks WHERE id = ?1");
    query_all(conn, &sql, &[&id])?.pop().ok_or(Error::NotFound)
}

fn query_all(conn: &Connection, sql: &str, params: &[&dyn ToSql]) -> Result<Vec<Task>, Error> {
    let mut statement = conn.prepare(sql)?;
    let mut rows = statement.query(params)?;
    let mut tasks = Vec::new();
    while let Some(row) = rows.next()? {
        tasks.push(from_row(row)?);
    }
    Ok(tasks)
}

fn from_row(row: &Row<'_>) -> Result<Task, Error> {
    let status: String = row.get(3)?;
    let kind: String = row.get(4)?;
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        status: TaskStatus::parse(&status)?,
        kind: TaskKind::parse(&kind)?,
        product_id: row.get(5)?,
        priority: row.get(6)?,
        branch: row.get(7)?,
        claimed_by: row.get(8)?,
        claim_id: row.get(9)?,
        claimed_at: row.get(10)?,
        claim_expires_at: row.get(11)?,
        commit_sha: row.get(12)?,
        verification: row.get(13)?,
        release_tag: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        merge_target_task_id: row.get(17)?,
        checks: decode_checks(row.get::<_, Option<String>>(18)?.as_deref())?,
    })
}

fn decode_checks(raw: Option<&str>) -> Result<Vec<Check>, Error> {
    match raw {
        Some(json) => Ok(serde_json::from_str(json)?),
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::{
        Check, NewTask, Releasable, TaskKind, TaskPatch, TaskStatus, available_transitions,
        can_transition, claim, create, get, issue_merge, list, list_active, list_by_status,
        merge_task_id, mergeable, pending_merges, releasable, release_product, report, set_status,
        set_status_by_operator, update,
    };
    use crate::clock::format_z;
    use crate::db::Db;
    use crate::error::Error;
    use crate::product::{self, Product};

    fn now() -> time::OffsetDateTime {
        datetime!(2026-03-04 05:06:07 UTC)
    }

    fn later() -> time::OffsetDateTime {
        datetime!(2026-03-04 05:06:08 UTC)
    }

    fn db_with_product() -> Db {
        let db = Db::open_in_memory().unwrap();
        product::upsert(
            &db,
            &Product {
                id: "a/b".into(),
                repository: "https://example.test/a/b.git".into(),
                description: String::new(),
                releases: true,
            },
            now(),
        )
        .unwrap();
        db
    }

    fn new_task(id: &str, kind: TaskKind, priority: i64) -> NewTask {
        NewTask {
            id: id.into(),
            title: format!("title {id}"),
            body: "body".into(),
            product_id: Some("a/b".into()),
            kind,
            priority,
        }
    }

    #[test]
    fn transition_table_matches_the_status_vocabulary() {
        use TaskStatus::{Blocked, Cancelled, Done, Draft, Dropped, Merged, Ready, Released, Wip};

        for (from, to) in [
            (Draft, Ready),
            (Ready, Wip),
            (Wip, Done),
            (Wip, Ready),
            (Done, Merged),
            (Merged, Released),
            (Blocked, Ready),
            (Draft, Blocked),
            (Merged, Cancelled),
            (Wip, Dropped),
        ] {
            assert!(can_transition(from, to), "{from:?} -> {to:?} must be legal");
        }

        for (from, to) in [
            (Draft, Wip),
            (Draft, Done),
            (Ready, Done),
            (Done, Released),
            (Released, Ready),
            (Released, Blocked),
            (Dropped, Ready),
            (Cancelled, Ready),
            (Ready, Ready),
            (Blocked, Blocked),
        ] {
            assert!(
                !can_transition(from, to),
                "{from:?} -> {to:?} must be denied"
            );
        }
    }

    #[test]
    fn create_starts_in_draft_and_rejects_blank_input() {
        let db = db_with_product();
        let task = create(&db, &new_task("t-1", TaskKind::Normal, 3), now()).unwrap();

        assert_eq!(task.status, TaskStatus::Draft);
        assert_eq!(task.kind, TaskKind::Normal);
        assert_eq!(task.priority, 3);
        assert_eq!(task.created_at, "2026-03-04T05:06:07Z");
        assert_eq!(task.created_at, task.updated_at);
        assert!(task.claim_id.is_none());
        assert_eq!(get(&db, "t-1").unwrap(), task);
        assert!(matches!(get(&db, "missing"), Err(Error::NotFound)));

        let mut blank = new_task("t-2", TaskKind::Normal, 0);
        blank.title = "  ".into();
        assert!(matches!(create(&db, &blank, now()), Err(Error::Invalid(_))));
    }

    #[test]
    fn listing_is_ordered_and_filterable_by_status() {
        let db = db_with_product();
        create(&db, &new_task("t-2", TaskKind::Normal, 0), now()).unwrap();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();

        let ids: Vec<String> = list(&db).unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["t-1", "t-2"]);

        let ready: Vec<String> = list_by_status(&db, TaskStatus::Ready)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ready, ["t-1"]);
    }

    #[test]
    fn claim_requires_a_worker_and_fills_the_lease() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();

        assert!(matches!(
            claim(&db, "  ", now(), 60),
            Err(Error::Invalid(_))
        ));
        assert!(claim(&db, "worker", now(), 60).unwrap().is_none());

        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();
        let leased = claim(&db, "worker", now(), 60).unwrap().unwrap();
        assert_eq!(leased.status, TaskStatus::Wip);
        assert_eq!(leased.claimed_at.as_deref(), Some("2026-03-04T05:06:07Z"));
        assert_eq!(
            leased.claim_expires_at.as_deref(),
            Some("2026-03-04T05:07:07Z")
        );
    }

    #[test]
    fn report_is_idempotent_for_the_same_commit() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();
        let leased = claim(&db, "worker", now(), 60).unwrap().unwrap();
        let claim_id = leased.claim_id.clone().unwrap();

        let done = report(&db, &claim_id, "abc1234", "cargo test", &[], now()).unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert_eq!(done.commit_sha.as_deref(), Some("abc1234"));

        let again = report(&db, &claim_id, "abc1234", "cargo test", &[], now()).unwrap();
        assert_eq!(again, done);

        assert!(matches!(
            report(&db, &claim_id, "def5678", "cargo test", &[], now()),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            report(&db, "not-a-claim", "abc1234", "cargo test", &[], now()),
            Err(Error::ClaimMismatch)
        ));
        assert!(matches!(
            report(&db, &claim_id, " ", "cargo test", &[], now()),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn tasks_without_a_product_cannot_be_released() {
        let db = db_with_product();
        let orphan = NewTask {
            product_id: None,
            ..new_task("t-1", TaskKind::Normal, 0)
        };
        create(&db, &orphan, now()).unwrap();

        // A task with no product never gets promoted in the first place.
        assert!(matches!(
            set_status(&db, "t-1", TaskStatus::Ready, now()),
            Err(Error::Precondition {
                code: "product_required",
                ..
            })
        ));

        // A row that got to `merged` before the gate existed keeps its status,
        // and is still refused a release: shipping needs a releasing product.
        force_status(&db, "t-1", "merged");
        assert!(matches!(
            set_status(&db, "t-1", TaskStatus::Released, now()),
            Err(Error::Invalid(_))
        ));
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);
    }

    /// Write a status straight to the row, standing in for a database written
    /// before the `ready` gate existed. Nothing in production does this.
    fn force_status(db: &Db, id: &str, status: &str) {
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE tasks SET status = ?2 WHERE id = ?1",
                rusqlite::params![id, status],
            )?;
            Ok(())
        })
        .unwrap();
    }

    /// Registration is open; promotion is gated. The refusal has to name the
    /// product and carry a code a client can branch on, and it must leave the
    /// task exactly where it was.
    #[test]
    fn ready_is_refused_while_the_product_is_not_catalogued() {
        let db = db_with_product();
        let unlisted = NewTask {
            product_id: Some("nobody/knows".into()),
            ..new_task("t-1", TaskKind::Normal, 0)
        };
        let created = create(&db, &unlisted, now()).expect("registration is not gated");
        assert_eq!(created.status, TaskStatus::Draft);
        assert!(
            available_transitions(&created).contains(&TaskStatus::Ready),
            "the table still offers ready; the gate refuses at the transition"
        );

        let err = set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap_err();
        assert!(
            matches!(&err, Error::Precondition { code, message }
                if *code == "product_not_catalogued" && message.contains("nobody/knows")),
            "unexpected error: {err:?}"
        );
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Draft);

        product::upsert(
            &db,
            &Product {
                id: "nobody/knows".into(),
                repository: "https://example.test/nobody/knows.git".into(),
                description: String::new(),
                releases: false,
            },
            now(),
        )
        .unwrap();
        assert_eq!(
            set_status(&db, "t-1", TaskStatus::Ready, now())
                .unwrap()
                .status,
            TaskStatus::Ready,
            "cataloguing the product is the whole remedy"
        );
    }

    /// The gate guards the promotion, not the row. Work that is already past
    /// `ready` is never demoted, and no other transition consults the catalogue.
    #[test]
    fn the_ready_gate_never_demotes_or_blocks_other_transitions() {
        let db = db_with_product();
        create(
            &db,
            &NewTask {
                product_id: Some("nobody/knows".into()),
                ..new_task("t-1", TaskKind::Normal, 0)
            },
            now(),
        )
        .unwrap();
        force_status(&db, "t-1", "wip");

        for to in [TaskStatus::Done, TaskStatus::Blocked] {
            let moved = set_status(&db, "t-1", to, now()).unwrap();
            assert_eq!(moved.status, to, "the gate must not touch {to:?}");
            force_status(&db, "t-1", "wip");
        }

        // Only the way back into `ready` is refused.
        assert!(matches!(
            set_status(&db, "t-1", TaskStatus::Ready, now()),
            Err(Error::Precondition {
                code: "product_not_catalogued",
                ..
            })
        ));
        assert_eq!(
            get(&db, "t-1").unwrap().status,
            TaskStatus::Wip,
            "a refused promotion leaves the row where it was"
        );
    }

    #[test]
    fn update_touches_only_the_fields_the_patch_carries() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 3), now()).unwrap();

        let patched = update(
            &db,
            "t-1",
            &TaskPatch {
                title: Some("renamed".into()),
                ..TaskPatch::default()
            },
            later(),
        )
        .unwrap();
        assert_eq!(patched.title, "renamed");
        assert_eq!(patched.body, "body");
        assert_eq!(patched.priority, 3);
        assert_eq!(patched.product_id.as_deref(), Some("a/b"));
        assert_eq!(patched.status, TaskStatus::Draft);
        assert_eq!(patched.created_at, "2026-03-04T05:06:07Z");
        assert_eq!(patched.updated_at, "2026-03-04T05:06:08Z");

        let moved = update(
            &db,
            "t-1",
            &TaskPatch {
                body: Some("new body".into()),
                priority: Some(9),
                branch: Some("feature/x".into()),
                ..TaskPatch::default()
            },
            later(),
        )
        .unwrap();
        assert_eq!(moved.title, "renamed");
        assert_eq!(moved.body, "new body");
        assert_eq!(moved.priority, 9);
        assert_eq!(moved.branch.as_deref(), Some("feature/x"));

        assert!(matches!(
            update(
                &db,
                "t-1",
                &TaskPatch {
                    title: Some("   ".into()),
                    ..TaskPatch::default()
                },
                now(),
            ),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            update(
                &db,
                "t-1",
                &TaskPatch {
                    product_id: Some("../etc/passwd".into()),
                    ..TaskPatch::default()
                },
                now(),
            ),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            update(&db, "missing", &TaskPatch::default(), now()),
            Err(Error::NotFound)
        ));
        assert_eq!(get(&db, "t-1").unwrap(), moved);
    }

    #[test]
    fn available_transitions_never_offer_merged_or_released() {
        let db = db_with_product();

        create(&db, &new_task("t-draft", TaskKind::Normal, 0), now()).unwrap();
        let draft = available_transitions(&get(&db, "t-draft").unwrap());
        assert_eq!(
            draft,
            vec![
                TaskStatus::Ready,
                TaskStatus::Blocked,
                TaskStatus::Cancelled,
                TaskStatus::Dropped,
            ]
        );

        create(&db, &new_task("t-ship", TaskKind::Normal, 0), now()).unwrap();
        for to in [TaskStatus::Ready, TaskStatus::Wip, TaskStatus::Done] {
            set_status(&db, "t-ship", to, now()).unwrap();
        }
        let done = available_transitions(&get(&db, "t-ship").unwrap());
        assert!(
            can_transition(TaskStatus::Done, TaskStatus::Merged),
            "the control plane still uses the transition table"
        );
        assert!(
            !done.contains(&TaskStatus::Merged),
            "merged is granted by a green merge report, never pressed: {done:?}"
        );
        assert!(done.contains(&TaskStatus::Blocked));

        set_status(&db, "t-ship", TaskStatus::Merged, now()).unwrap();
        let merged = available_transitions(&get(&db, "t-ship").unwrap());
        assert!(
            can_transition(TaskStatus::Merged, TaskStatus::Released),
            "the control plane still uses the transition table"
        );
        assert!(
            !merged.contains(&TaskStatus::Released),
            "released is granted by a product release, never pressed: {merged:?}"
        );
        assert!(merged.contains(&TaskStatus::Cancelled));

        set_status(&db, "t-ship", TaskStatus::Released, now()).unwrap();
        assert!(available_transitions(&get(&db, "t-ship").unwrap()).is_empty());
    }

    #[test]
    fn claim_derives_a_branch_from_the_task_id_and_keeps_an_existing_one() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        create(&db, &new_task("t-2", TaskKind::Normal, 0), now()).unwrap();
        update(
            &db,
            "t-2",
            &TaskPatch {
                branch: Some("feature/manual".into()),
                ..TaskPatch::default()
            },
            now(),
        )
        .unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();
        set_status(&db, "t-2", TaskStatus::Ready, now()).unwrap();

        let first = claim(&db, "worker", now(), 60).unwrap().unwrap();
        assert_eq!(first.id, "t-1");
        assert_eq!(first.branch.as_deref(), Some("task/t-1"));

        let second = claim(&db, "worker", now(), 60).unwrap().unwrap();
        assert_eq!(second.id, "t-2");
        assert_eq!(second.branch.as_deref(), Some("feature/manual"));
    }

    #[test]
    fn an_expired_lease_is_reclaimed_with_a_fresh_claim_id() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();

        let first = claim(&db, "worker-a", now(), 60).unwrap().unwrap();
        let abandoned = first.claim_id.clone().unwrap();
        assert_eq!(
            first.claim_expires_at.as_deref(),
            Some("2026-03-04T05:07:07Z")
        );

        // A lease that has not expired yet belongs to the worker holding it.
        let alive = now() + time::Duration::seconds(59);
        assert!(claim(&db, "worker-b", alive, 60).unwrap().is_none());

        let expired = now() + time::Duration::seconds(61);
        let retaken = claim(&db, "worker-b", expired, 60).unwrap().unwrap();
        assert_eq!(retaken.id, "t-1");
        assert_eq!(retaken.status, TaskStatus::Wip);
        assert_eq!(retaken.claimed_by.as_deref(), Some("worker-b"));
        assert_eq!(retaken.claimed_at.as_deref(), Some("2026-03-04T05:07:08Z"));
        assert_eq!(
            retaken.claim_expires_at.as_deref(),
            Some("2026-03-04T05:08:08Z")
        );
        let fresh = retaken.claim_id.clone().unwrap();
        assert_ne!(fresh, abandoned, "a reclaim must issue a new claim_id");

        assert!(
            matches!(
                report(&db, &abandoned, "abc1234", "cargo test", &[], expired),
                Err(Error::ClaimMismatch)
            ),
            "the abandoned lease must no longer report"
        );
        assert_eq!(
            report(&db, &fresh, "abc1234", "cargo test", &[], expired)
                .unwrap()
                .status,
            TaskStatus::Done
        );

        // A task that left `wip` is never handed out again by expiry.
        let far_future = now() + time::Duration::seconds(100_000);
        assert!(claim(&db, "worker-c", far_future, 60).unwrap().is_none());
    }

    #[test]
    fn a_wip_task_without_a_lease_is_never_reclaimed() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();
        // Moved by a human, so no lease columns were ever written.
        set_status(&db, "t-1", TaskStatus::Wip, now()).unwrap();
        assert!(get(&db, "t-1").unwrap().claim_expires_at.is_none());

        let far_future = now() + time::Duration::seconds(100_000);
        assert!(
            claim(&db, "worker", far_future, 60).unwrap().is_none(),
            "wip without claim_expires_at has no expiry to pass"
        );
    }

    #[test]
    fn list_active_hides_released_tasks() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        create(&db, &new_task("t-2", TaskKind::Normal, 0), now()).unwrap();
        for to in [
            TaskStatus::Ready,
            TaskStatus::Wip,
            TaskStatus::Done,
            TaskStatus::Merged,
            TaskStatus::Released,
        ] {
            set_status(&db, "t-1", to, now()).unwrap();
        }

        let ids: Vec<String> = list_active(&db)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["t-2"]);
        assert_eq!(list(&db).unwrap().len(), 2);
        assert_eq!(
            list_by_status(&db, TaskStatus::Released).unwrap()[0].id,
            "t-1"
        );
    }

    fn green() -> Vec<Check> {
        vec![Check {
            name: "cargo test".into(),
            exit_code: 0,
        }]
    }

    /// Take `id` from draft to done the way a worker does.
    fn work_to_done(db: &Db, id: &str) {
        set_status(db, id, TaskStatus::Ready, now()).unwrap();
        let leased = claim(db, "worker", now(), 60).unwrap().unwrap();
        assert_eq!(leased.id, id);
        let claim_id = leased.claim_id.expect("claim_id");
        report(db, &claim_id, "abc1234", "cargo test", &[], now()).unwrap();
    }

    #[test]
    fn only_finished_normal_work_without_a_live_merge_is_mergeable() {
        let db = db_with_product();
        for id in ["t-done", "t-ready", "t-draft"] {
            create(&db, &new_task(id, TaskKind::Normal, 0), now()).unwrap();
        }
        set_status(&db, "t-ready", TaskStatus::Ready, now()).unwrap();
        work_to_done(&db, "t-done");

        let ids: Vec<String> = mergeable(&db).unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["t-done"]);
        assert!(pending_merges(&db).unwrap().is_empty());

        let merge = issue_merge(&db, "t-done", later()).unwrap();
        assert_eq!(merge.id, merge_task_id("t-done"));
        assert_eq!(merge.kind, TaskKind::InstantMerge);
        assert_eq!(merge.status, TaskStatus::Ready);
        assert_eq!(merge.merge_target_task_id.as_deref(), Some("t-done"));
        assert_eq!(merge.product_id.as_deref(), Some("a/b"));
        assert_eq!(merge.branch.as_deref(), Some("task/t-done"));
        assert_eq!(merge.commit_sha.as_deref(), Some("abc1234"));
        assert!(merge.title.contains("t-done"));

        assert!(
            mergeable(&db).unwrap().is_empty(),
            "a live merge takes its target out of the candidate list"
        );
        let pending: Vec<String> = pending_merges(&db)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(pending, [merge_task_id("t-done")]);

        assert!(
            matches!(issue_merge(&db, "t-done", later()), Err(Error::Conflict(_))),
            "a second live merge for one target is a conflict"
        );
        assert!(matches!(
            issue_merge(&db, "t-ready", later()),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            issue_merge(&db, "t-missing", later()),
            Err(Error::NotFound)
        ));

        // A dropped attempt frees the target again.
        set_status(&db, &merge_task_id("t-done"), TaskStatus::Dropped, later()).unwrap();
        let ids: Vec<String> = mergeable(&db).unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            ["t-done"],
            "a dropped merge no longer holds its target"
        );
    }

    #[test]
    fn a_merge_lands_its_target_only_on_green_checks() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let merge = issue_merge(&db, "t-1", later()).unwrap();
        let leased = claim(&db, "worker", later(), 60).unwrap().unwrap();
        assert_eq!(leased.id, merge.id);
        let claim_id = leased.claim_id.expect("claim_id");

        for checks in [
            Vec::new(),
            vec![Check {
                name: "cargo test".into(),
                exit_code: 101,
            }],
        ] {
            assert!(
                matches!(
                    report(&db, &claim_id, "abc1234", "cargo test", &checks, later()),
                    Err(Error::Invalid(_))
                ),
                "{checks:?} must not land"
            );
            assert_eq!(get(&db, &merge.id).unwrap().status, TaskStatus::Wip);
            assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Done);
        }

        let landed = report(&db, &claim_id, "abc1234", "cargo test", &green(), later()).unwrap();
        assert_eq!(landed.status, TaskStatus::Done);
        assert_eq!(landed.checks, green(), "the evidence is kept on the task");
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);

        // Idempotent: the same commit reported twice is still accepted.
        let again = report(&db, &claim_id, "abc1234", "cargo test", &green(), later()).unwrap();
        assert_eq!(again, landed);
    }

    /// The gate is not a one-time door. Once a merge is `done`, a repeat report
    /// still has to carry the same evidence, or a worker could report "no
    /// checks" against a landed merge and read the 200 as a pass.
    #[test]
    fn a_landed_merge_still_refuses_a_report_without_green_checks() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let merge = issue_merge(&db, "t-1", later()).unwrap();
        let claim_id = claim(&db, "worker", later(), 60)
            .unwrap()
            .unwrap()
            .claim_id
            .expect("claim_id");
        let landed = report(&db, &claim_id, "abc1234", "cargo test", &green(), later()).unwrap();
        assert_eq!(landed.status, TaskStatus::Done);
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);

        for checks in [
            Vec::new(),
            vec![Check {
                name: "cargo test".into(),
                exit_code: 101,
            }],
        ] {
            assert!(
                matches!(
                    report(&db, &claim_id, "abc1234", "cargo test", &checks, later()),
                    Err(Error::Invalid(_))
                ),
                "{checks:?} must not pass the gate on a landed merge"
            );
            assert_eq!(
                get(&db, &merge.id).unwrap(),
                landed,
                "a refused repeat must not touch the merge"
            );
            assert_eq!(
                get(&db, "t-1").unwrap().status,
                TaskStatus::Merged,
                "a refused repeat must not touch the target"
            );
        }

        let again = report(&db, &claim_id, "abc1234", "cargo test", &green(), later()).unwrap();
        assert_eq!(again, landed, "a green repeat is still idempotent");
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);
    }

    /// The index frees a target once its merge is cancelled, so the id rule has
    /// to leave room for the retry the index allows.
    #[test]
    fn a_cancelled_merge_is_reissued_under_a_new_id() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");

        let first = issue_merge(&db, "t-1", later()).unwrap();
        assert_eq!(first.id, merge_task_id("t-1"));
        assert!(
            matches!(issue_merge(&db, "t-1", later()), Err(Error::Conflict(_))),
            "a live merge still blocks a second issue"
        );

        set_status(&db, &first.id, TaskStatus::Cancelled, later()).unwrap();
        let second = issue_merge(&db, "t-1", later()).unwrap();
        assert_ne!(second.id, first.id, "the retry needs an id of its own");
        assert!(
            !second.id.contains('/'),
            "a task id is one path segment: {}",
            second.id
        );
        assert_eq!(second.status, TaskStatus::Ready);
        assert_eq!(second.kind, TaskKind::InstantMerge);
        assert_eq!(second.merge_target_task_id.as_deref(), Some("t-1"));
        assert_eq!(second.product_id.as_deref(), Some("a/b"));
        assert_eq!(second.branch.as_deref(), Some("task/t-1"));
        assert_eq!(second.commit_sha.as_deref(), Some("abc1234"));
        assert_eq!(get(&db, &first.id).unwrap().status, TaskStatus::Cancelled);

        assert!(
            matches!(issue_merge(&db, "t-1", later()), Err(Error::Conflict(_))),
            "the retry is itself the one live merge now"
        );
        set_status(&db, &second.id, TaskStatus::Dropped, later()).unwrap();
        let third = issue_merge(&db, "t-1", later()).unwrap();
        assert_ne!(third.id, first.id);
        assert_ne!(third.id, second.id);
    }

    #[test]
    fn a_merge_whose_target_already_moved_lands_nothing() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let merge = issue_merge(&db, "t-1", later()).unwrap();
        let leased = claim(&db, "worker", later(), 60).unwrap().unwrap();
        let claim_id = leased.claim_id.expect("claim_id");

        set_status(&db, "t-1", TaskStatus::Merged, later()).unwrap();
        assert!(matches!(
            report(&db, &claim_id, "abc1234", "cargo test", &green(), later()),
            Err(Error::Invalid(_))
        ));
        assert_eq!(
            get(&db, &merge.id).unwrap().status,
            TaskStatus::Wip,
            "the refused report must roll the merge back too"
        );
    }

    #[test]
    fn a_normal_report_keeps_its_checks_and_needs_none() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();
        let claim_id = claim(&db, "worker", now(), 60)
            .unwrap()
            .unwrap()
            .claim_id
            .unwrap();

        let done = report(&db, &claim_id, "abc1234", "cargo test", &green(), now()).unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert_eq!(done.checks, green());
        assert_eq!(get(&db, "t-1").unwrap().checks, green());
    }

    #[test]
    fn release_stamps_only_the_merged_tasks_of_a_releasing_product() {
        let db = db_with_product();
        product::upsert(
            &db,
            &Product {
                id: "c/d".into(),
                repository: "https://example.test/c/d.git".into(),
                description: String::new(),
                releases: false,
            },
            now(),
        )
        .unwrap();

        for (id, product_id) in [
            ("t-ship-1", "a/b"),
            ("t-ship-2", "a/b"),
            ("t-open", "a/b"),
            ("t-keep", "c/d"),
        ] {
            create(
                &db,
                &NewTask {
                    product_id: Some(product_id.into()),
                    ..new_task(id, TaskKind::Normal, 0)
                },
                now(),
            )
            .unwrap();
        }
        for id in ["t-ship-1", "t-ship-2", "t-keep"] {
            for to in [
                TaskStatus::Ready,
                TaskStatus::Wip,
                TaskStatus::Done,
                TaskStatus::Merged,
            ] {
                set_status(&db, id, to, now()).unwrap();
            }
        }
        for to in [TaskStatus::Ready, TaskStatus::Wip, TaskStatus::Done] {
            set_status(&db, "t-open", to, now()).unwrap();
        }

        assert_eq!(
            releasable(&db).unwrap(),
            vec![Releasable {
                product_id: "a/b".into(),
                task_count: 2,
            }],
            "a product that does not release never queues one"
        );

        assert!(matches!(
            release_product(&db, "c/d", "v1", later()),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            release_product(&db, "a/b", "  ", later()),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            release_product(&db, "x/y", "v1", later()),
            Err(Error::NotFound)
        ));

        let released = release_product(&db, "a/b", "v0.2.0", later()).unwrap();
        let ids: Vec<String> = released.iter().map(|t| t.id.clone()).collect();
        assert_eq!(ids, ["t-ship-1", "t-ship-2"]);
        for task in &released {
            assert_eq!(task.status, TaskStatus::Released);
            assert_eq!(task.release_tag.as_deref(), Some("v0.2.0"));
            assert_eq!(task.updated_at, "2026-03-04T05:06:08Z");
        }

        assert_eq!(get(&db, "t-open").unwrap().status, TaskStatus::Done);
        assert!(get(&db, "t-open").unwrap().release_tag.is_none());
        assert_eq!(get(&db, "t-keep").unwrap().status, TaskStatus::Merged);
        assert!(get(&db, "t-keep").unwrap().release_tag.is_none());

        assert!(releasable(&db).unwrap().is_empty());
        assert!(
            matches!(
                release_product(&db, "a/b", "v0.2.1", later()),
                Err(Error::Conflict(_))
            ),
            "a release with nothing merged is a conflict"
        );
        assert_eq!(
            get(&db, "t-ship-1").unwrap().release_tag.as_deref(),
            Some("v0.2.0"),
            "the refused release must not have restamped anything"
        );
    }

    /// Registration files ordinary work. A merge is issued by the control plane
    /// against a task it can name, and a hand-made one would be a merge with no
    /// target: claimed ahead of everything and impossible to report.
    #[test]
    fn create_refuses_an_instant_merge_while_the_control_plane_still_issues_one() {
        let db = db_with_product();

        let refused = create(&db, &new_task("t-forged", TaskKind::InstantMerge, 0), now())
            .expect_err("a hand-made merge must be refused");
        assert!(
            matches!(&refused, Error::Invalid(message) if message.contains("/api/merges")),
            "the refusal must point at the control plane: {refused:?}"
        );
        assert!(
            matches!(get(&db, "t-forged"), Err(Error::NotFound)),
            "a refused creation writes no row"
        );

        // The internal path is untouched: a merge still comes from a target.
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let merge = issue_merge(&db, "t-1", later()).unwrap();
        assert_eq!(merge.kind, TaskKind::InstantMerge);
        assert_eq!(merge.merge_target_task_id.as_deref(), Some("t-1"));
    }

    /// The last line of defence, kept because it is the reason `create` refuses:
    /// a merge row with no target can never be reported, whatever put it there.
    #[test]
    fn a_merge_without_a_target_lands_nothing() {
        let db = db_with_product();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, kind, product_id, priority,
                                    created_at, updated_at)
                 VALUES ('t-orphan', 'orphan merge', 'ready', 'instant:merge', 'a/b', 0, ?1, ?1)",
                [format_z(now())],
            )?;
            Ok(())
        })
        .unwrap();

        let leased = claim(&db, "worker", now(), 60).unwrap().unwrap();
        assert_eq!(leased.id, "t-orphan");
        let claim_id = leased.claim_id.expect("claim_id");

        let refused = report(&db, &claim_id, "abc1234", "cargo test", &green(), later())
            .expect_err("a merge with no target lands nothing");
        assert!(
            matches!(&refused, Error::Invalid(message) if message.contains("no target")),
            "unexpected error: {refused:?}"
        );
        assert_eq!(
            get(&db, "t-orphan").unwrap().status,
            TaskStatus::Wip,
            "the refused report must roll the merge back"
        );
    }

    /// One rule, one place: every human surface — HTTP and MCP alike — refuses
    /// `merged` and `released`, while the control plane goes on using
    /// `set_status`.
    #[test]
    fn an_operator_may_not_press_merged_or_released() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");

        for to in [TaskStatus::Merged, TaskStatus::Released] {
            let refused = set_status_by_operator(&db, "t-1", to, later())
                .expect_err("an operator may not press it");
            assert!(
                matches!(&refused, Error::Invalid(message) if message.contains("control plane")),
                "the refusal must name the control plane: {refused:?}"
            );
            assert_eq!(
                get(&db, "t-1").unwrap().status,
                TaskStatus::Done,
                "a refusal moves no row"
            );
        }

        // Everything else still goes through, and the control plane still lands
        // the task through `set_status`.
        let blocked = set_status_by_operator(&db, "t-1", TaskStatus::Blocked, later()).unwrap();
        assert_eq!(blocked.status, TaskStatus::Blocked);
        set_status(&db, "t-1", TaskStatus::Ready, later()).unwrap();
        set_status(&db, "t-1", TaskStatus::Wip, later()).unwrap();
        set_status(&db, "t-1", TaskStatus::Done, later()).unwrap();
        let merged = set_status(&db, "t-1", TaskStatus::Merged, later()).unwrap();
        assert_eq!(merged.status, TaskStatus::Merged);
    }
}
