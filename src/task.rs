use rusqlite::{Connection, Row, ToSql};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::clock::format_z;
use crate::db::Db;
use crate::error::Error;
use crate::product::{self, check_product_id};

const COLUMNS: &str = "id, title, body, status, kind, product_id, priority, branch, claimed_by, \
                       claim_id, claimed_at, claim_expires_at, commit_sha, verification, \
                       release_tag, created_at, updated_at, merge_target_task_id, checks_json, \
                       review_target_task_id, review_verdict, release_level, release_task_id, \
                       depends_on, done_at";

/// Every status, in vocabulary order. Used to enumerate legal transitions.
pub(crate) const ALL_STATUSES: [TaskStatus; 10] = [
    TaskStatus::Draft,
    TaskStatus::Ready,
    TaskStatus::Wip,
    TaskStatus::Done,
    TaskStatus::Approved,
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
    /// A reviewer read the commit the work reported and approved it. Granted by
    /// an approving review report alone, never pressed by a human.
    Approved,
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
            Self::Approved => "approved",
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
            "approved" => Ok(Self::Approved),
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
    /// A reading of finished work, answered with a verdict rather than a commit.
    #[serde(rename = "review")]
    Review,
    /// The shipping of one product's landed work: a worker bumps the version
    /// at the card's `release_level` and reports the tag it cut.
    #[serde(rename = "instant:release")]
    InstantRelease,
}

impl TaskKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::InstantMerge => "instant:merge",
            Self::Review => "review",
            Self::InstantRelease => "instant:release",
        }
    }

    /// The control plane route that issues this kind, for a refusal that can
    /// say where the task should have come from. Ordinary work has none.
    fn issued_by(self) -> Option<&'static str> {
        match self {
            Self::Normal => None,
            Self::InstantMerge => Some("POST /api/merges"),
            Self::Review => Some("POST /api/reviews"),
            Self::InstantRelease => Some("POST /api/releases"),
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "normal" => Ok(Self::Normal),
            "instant:merge" => Ok(Self::InstantMerge),
            "review" => Ok(Self::Review),
            "instant:release" => Ok(Self::InstantRelease),
            other => Err(Error::Invalid(format!("invalid kind: {other}"))),
        }
    }
}

/// How much a release of this work moves the version: the semver component a
/// `bump-tag` run steps. Known when the work is filed, which is why it lives on
/// the task rather than being asked for at release time. The order is the
/// order of the components, so the level of a release is the largest level of
/// the work it ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseLevel {
    #[default]
    Patch,
    Minor,
    Major,
}

impl ReleaseLevel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "patch" => Ok(Self::Patch),
            "minor" => Ok(Self::Minor),
            "major" => Ok(Self::Major),
            other => Err(Error::Invalid(format!(
                "invalid release_level: {other} (one of patch, minor, major)"
            ))),
        }
    }

    /// The default when nothing was said, and what an absent value decodes to.
    pub fn parse_optional(raw: Option<&str>) -> Result<Self, Error> {
        raw.map_or(Ok(Self::Patch), Self::parse)
    }
}

/// The shape a release tag has to take: `v<major>.<minor>.<patch>`. A worker
/// reports the tag `bump-tag` cut, and a tag that is not one is not a release.
fn is_release_tag(raw: &str) -> bool {
    let Some(rest) = raw.strip_prefix('v') else {
        return false;
    };
    let parts: Vec<&str> = rest.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// How a review answered the work it read.
///
/// `RequestChanges` is a finished review, not a failed one: the reviewer did
/// their job and the answer is "not yet", so it is reported as a success and
/// carries the same findings an approval does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approve,
    RequestChanges,
}

impl ReviewVerdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::RequestChanges => "request_changes",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "approve" => Ok(Self::Approve),
            "request_changes" => Ok(Self::RequestChanges),
            other => Err(Error::Invalid(format!("invalid verdict: {other}"))),
        }
    }
}

/// What a task's latest finished review said, read from that review's own row.
/// Derived on the way out, so the verdict and the findings live in exactly one
/// place: the review task that reported them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewOutcome {
    pub review_task_id: String,
    pub verdict: ReviewVerdict,
    /// What the reviewer wrote, kept in the review's `verification`.
    pub findings: Option<String>,
    /// The commit the review was issued for and answered about.
    pub subject_commit_sha: Option<String>,
    pub reported_at: String,
}

/// How a worker's report ends the task it was leased for.
///
/// The default is [`Self::Done`], which is the report every worker written
/// before this existed sends, so leaving the field out keeps the old contract
/// exactly. [`Self::Blocked`] is the worker saying it could not finish — a
/// rebase that conflicted, a check that failed — and it is a *successful*
/// report of a failure: the reason and the checks are written down and kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportOutcome {
    Done,
    Blocked,
}

impl ReportOutcome {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Blocked => "blocked",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "done" => Ok(Self::Done),
            "blocked" => Ok(Self::Blocked),
            other => Err(Error::Invalid(format!("invalid outcome: {other}"))),
        }
    }

    /// The outcome of a report whose `outcome` field is optional on the wire.
    ///
    /// Every worker surface takes it that way — HTTP `/worker/report` and the
    /// MCP `task_report` tool — and both mean the same two things by it: an
    /// omitted outcome is [`Self::Done`], the report a worker written before
    /// outcomes existed sends, and anything that is not one of the two names is
    /// refused rather than quietly read as success. That is one transport
    /// contract, so it is owned here rather than restated at each surface,
    /// where the two could drift into disagreeing about what silence means.
    pub fn parse_optional(raw: Option<&str>) -> Result<Self, Error> {
        raw.map_or(Ok(Self::Done), Self::parse)
    }
}

/// One verification a worker ran before asking for a merge. `exit_code` is the
/// process status, so `0` is the only pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub exit_code: i64,
}

/// A product with landed work that no release is carrying.
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
    /// Set on a `review` task: the task this review reads. Kept apart from
    /// `merge_target_task_id` because the two are live for different spans — a
    /// finished review frees its target, a landed merge keeps it — and one
    /// column could not carry both partial unique indexes.
    #[serde(default)]
    pub review_target_task_id: Option<String>,
    /// Set on a `review` task once it answered.
    #[serde(default)]
    pub review_verdict: Option<ReviewVerdict>,
    /// Decoded from `checks_json`; the column itself is never exposed.
    #[serde(default)]
    pub checks: Vec<Check>,
    /// How far a release of this work steps the version. On a subtask, inherited
    /// from its target at issue; on a release task, the largest level it ships.
    #[serde(default)]
    pub release_level: ReleaseLevel,
    /// Set on a `normal` task once a release task was issued to ship it.
    #[serde(default)]
    pub release_task_id: Option<String>,
    /// The task this one waits for. While it is set and that task has not
    /// landed (`merged` or `released`), this one is not promoted to `ready`;
    /// the landing promotes it. One id, not a list: a chain expresses more.
    #[serde(default)]
    pub depends_on: Option<String>,
    /// The moment a `normal` task first reached `done`, written once and never
    /// overwritten by a later transition (approval, landing, release, or a
    /// second pass through `done` after `request_changes`). `None` on a task
    /// that has never been `done`, and always `None` on a `review` or
    /// `instant:merge` task — the done screen this exists for reads `normal`
    /// work only.
    #[serde(default)]
    pub done_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTask {
    pub id: String,
    pub title: String,
    pub body: String,
    pub product_id: Option<String>,
    pub kind: TaskKind,
    pub priority: i64,
    #[serde(default)]
    pub release_level: ReleaseLevel,
    #[serde(default)]
    pub depends_on: Option<String>,
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
    pub release_level: Option<ReleaseLevel>,
    /// Absent leaves the dependency alone; an explicit `null` clears it; an
    /// id sets it. The two nulls have to stay apart, because clearing a
    /// dependency is how a person skips the order on purpose.
    #[serde(deserialize_with = "double_option")]
    pub depends_on: Option<Option<String>>,
}

/// `null` and "not sent" are different answers for an optional field that can
/// be cleared: the outer `Option` is presence, the inner one is the value.
pub fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// Create a task in `draft`.
///
/// Registration files ordinary work only. A merge or a review is issued by the
/// control plane against a target it can name, and a hand-made one would have no
/// target: claimed like any other task, impossible to report, and so a standing
/// block on the queue. The refusal lives here rather than in a transport, so
/// HTTP and MCP cannot drift apart on it.
pub fn create(db: &Db, new: &NewTask, now: OffsetDateTime) -> Result<Task, Error> {
    if let Some(route) = new.kind.issued_by() {
        return Err(Error::Invalid(format!(
            "a {} task is issued by the control plane ({route}) against the task it answers \
             for, and cannot be created directly",
            new.kind.as_str()
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
    db.with_tx(|tx| {
        check_dependency(tx, &new.id, new.depends_on.as_deref())?;
        tx.execute(
            "INSERT INTO tasks (id, title, body, status, kind, product_id, priority, release_level,
                                depends_on, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            rusqlite::params![
                new.id,
                new.title,
                new.body,
                TaskStatus::Draft.as_str(),
                new.kind.as_str(),
                new.product_id,
                new.priority,
                new.release_level.as_str(),
                new.depends_on,
                stamp,
            ],
        )?;
        read(tx, &new.id)
    })
}

/// Whether `depends_on` may be the dependency of `id`.
///
/// A dependency that could never land is refused at the door rather than
/// left to strand the task: itself, a task that does not exist, one that was
/// called off, or one whose own chain leads back here (`A → B → A` would wait
/// for ever, and one step at a time is enough to find it).
fn check_dependency(conn: &Connection, id: &str, depends_on: Option<&str>) -> Result<(), Error> {
    let Some(dependency) = depends_on else {
        return Ok(());
    };
    if dependency == id {
        return Err(Error::Invalid(format!("task {id} cannot depend on itself")));
    }
    let target = match read(conn, dependency) {
        Ok(target) => target,
        Err(Error::NotFound) => {
            return Err(Error::Invalid(format!(
                "task {id} cannot depend on {dependency}: no such task"
            )));
        }
        Err(other) => return Err(other),
    };
    if matches!(target.status, TaskStatus::Cancelled | TaskStatus::Dropped) {
        return Err(Error::Invalid(format!(
            "task {id} cannot depend on {dependency}: it is {} and will never land",
            target.status.as_str()
        )));
    }
    // Walk the chain from the dependency. Reaching `id` again is a cycle;
    // a chain longer than the table is one too, however it got there.
    let mut seen = std::collections::HashSet::new();
    let mut next = target.depends_on;
    while let Some(link) = next {
        if link == id {
            return Err(Error::Invalid(format!(
                "task {id} cannot depend on {dependency}: that chain leads back to {id}"
            )));
        }
        if !seen.insert(link.clone()) {
            return Err(Error::Invalid(format!(
                "task {id} cannot depend on {dependency}: its chain loops at {link}"
            )));
        }
        next = match read(conn, &link) {
            Ok(task) => task.depends_on,
            Err(Error::NotFound) => None,
            Err(other) => return Err(other),
        };
    }
    Ok(())
}

/// Whether a dependency has landed: `merged` or `released`, and nothing else.
#[must_use]
pub fn has_landed(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Merged | TaskStatus::Released)
}

/// The dependency `task` is still waiting for, with its status; `None` when it
/// has none or it has landed.
fn pending_dependency(
    conn: &Connection,
    task: &Task,
) -> Result<Option<(String, TaskStatus)>, Error> {
    let Some(dependency) = task.depends_on.as_deref() else {
        return Ok(None);
    };
    let target = read(conn, dependency)?;
    if has_landed(target.status) {
        return Ok(None);
    }
    Ok(Some((target.id, target.status)))
}

/// The status of the dependency `task` is still waiting for, read for the
/// card. `None` when it has none or it has landed.
pub fn dependency_status(db: &Db, task: &Task) -> Result<Option<TaskStatus>, Error> {
    db.with_conn(|conn| Ok(pending_dependency(conn, task)?.map(|(_, status)| status)))
}

/// `landed_id` just landed: promote every `draft` task that waited for it.
///
/// The promotion goes through the same gate a pressed `ready` does, so a task
/// whose product left the catalogue stays `draft` — and says why on its
/// `verification`, because a task that silently stays `draft` is exactly what
/// this column exists to end. `blocked` tasks are a person's decision and are
/// left alone.
fn promote_dependants(tx: &Connection, landed_id: &str, stamp: &str) -> Result<(), Error> {
    let waiting = query_all(
        tx,
        &format!(
            "SELECT {COLUMNS} FROM tasks WHERE depends_on = ?1 AND status = 'draft'
             ORDER BY created_at ASC, id ASC"
        ),
        &[&landed_id],
    )?;
    for task in waiting {
        match check_catalogued(tx, &task) {
            Ok(()) => {
                tx.execute(
                    "UPDATE tasks SET status = 'ready', updated_at = ?2 WHERE id = ?1",
                    rusqlite::params![task.id, stamp],
                )?;
            }
            Err(refusal) => {
                tx.execute(
                    "UPDATE tasks SET verification = ?2, updated_at = ?3 WHERE id = ?1",
                    rusqlite::params![
                        task.id,
                        format!("not promoted when {landed_id} landed: {refusal}"),
                        stamp
                    ],
                )?;
            }
        }
    }
    Ok(())
}

/// `called_off_id` was cancelled or dropped: the tasks waiting for it will
/// never be promoted, so they are blocked with the reason on the row rather
/// than left `draft` for ever. Work already under way is not touched.
fn block_dependants(
    tx: &Connection,
    called_off_id: &str,
    status: TaskStatus,
    stamp: &str,
) -> Result<(), Error> {
    tx.execute(
        "UPDATE tasks SET status = 'blocked', verification = ?2, updated_at = ?3
         WHERE depends_on = ?1 AND status IN ('draft', 'ready')",
        rusqlite::params![
            called_off_id,
            format!(
                "blocked: depends on {called_off_id}, which was {} and will never land",
                status.as_str()
            ),
            stamp
        ],
    )?;
    Ok(())
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

/// Completed `normal` work, most recently finished first: everything that has
/// passed `done` and not fallen back out of it (`done`, `approved`, `merged`,
/// `released`). `review` and `instant:merge` tasks never carry a `done_at` for
/// their own kind, so restricting to `normal` and ordering by `done_at` are
/// the same filter stated twice; both are named because the id tiebreak needs
/// a `normal`-only ordering to be meaningful at all.
pub fn list_done(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks
                 WHERE kind = 'normal' AND status IN ('done', 'approved', 'merged', 'released')
                 ORDER BY done_at DESC, id DESC"
            ),
            &[],
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
        let depends_on = match &patch.depends_on {
            Some(next) => next.as_deref(),
            None => task.depends_on.as_deref(),
        };
        if patch.depends_on.is_some() {
            check_dependency(tx, id, depends_on)?;
        }
        tx.execute(
            "UPDATE tasks SET title = ?2, body = ?3, product_id = ?4, priority = ?5, branch = ?6,
                    release_level = ?7, depends_on = ?8, updated_at = ?9
             WHERE id = ?1",
            rusqlite::params![
                id,
                patch.title.as_deref().unwrap_or(&task.title),
                patch.body.as_deref().unwrap_or(&task.body),
                patch.product_id.as_deref().or(task.product_id.as_deref()),
                patch.priority.unwrap_or(task.priority),
                patch.branch.as_deref().or(task.branch.as_deref()),
                patch.release_level.unwrap_or(task.release_level).as_str(),
                depends_on,
                stamp,
            ],
        )?;
        read(tx, id)
    })
}

/// The statuses a human may actually press on `task`.
///
/// `approved`, `merged`, and `released` are deliberately absent: work is
/// approved only by a review that read it, it lands only when a merge reported
/// green checks, and it ships only through a product release. So is `done` on a
/// review, which is finished by its verdict. The transition table itself still
/// allows all of them, because the control plane goes through it.
///
/// A review that already answered offers nothing at all: it is the record of a
/// verdict, and every press off it would either reopen the answer or file the
/// finished attempt as abandoned.
///
/// A merge offers neither `done` nor `blocked`: how an attempt ended is the
/// worker's report, which carries the checks and lands the target. A blocked
/// merge offers no `ready` either: an attempt that could not be integrated is
/// called off and reissued, never restarted. What is left on a merge is `wip`
/// and the two presses that call the attempt off.
///
/// The list and the refusal are the same rule read two ways — see
/// [`operator_refusal`] — so what a surface offers can never drift from what it
/// accepts.
#[must_use]
pub fn available_transitions(task: &Task) -> Vec<TaskStatus> {
    ALL_STATUSES
        .into_iter()
        .filter(|&to| operator_refusal(task, to).is_none())
        .filter(|&to| can_transition(task.status, to))
        .collect()
}

/// Why an operator surface may not press `to` on `task`, if it may not.
///
/// The single owner of "a human may not press this", consulted both by
/// [`available_transitions`] and by [`set_status_by_operator`].
fn operator_refusal(task: &Task, to: TaskStatus) -> Option<Error> {
    if matches!(
        to,
        TaskStatus::Approved | TaskStatus::Merged | TaskStatus::Released
    ) {
        return Some(Error::Invalid(format!(
            "{} is granted by the control plane (POST /api/reviews, POST /api/merges, \
             POST /api/releases), not by a status change",
            to.as_str()
        )));
    }
    // A review that answered is the record of that answer, and no press moves
    // it again. Every operator transition off it is refused, because each of the
    // three the table would still allow reopens something that is closed:
    //
    //   * `blocked` puts the finished attempt back inside the single-open-review
    //     index — its predicate is `status NOT IN ('done', 'cancelled',
    //     'dropped', 'released')` — so an attempt that is over would stand in
    //     the way of the next review of the same target;
    //   * from `blocked` the row walks on to `ready` and, on a claim, to `wip`,
    //     where `review_report` accepts it again and writes a second verdict
    //     over the first — the one answer the target lived by, overwritten by
    //     hand;
    //   * `cancelled` and `dropped` would file an answered review as abandoned,
    //     which is a lie about a review that did its job.
    //
    // "Answered" is either mark, not both. `review_report` writes `status =
    // 'done'` and `review_verdict` in one UPDATE inside one transaction, so the
    // two are always written together and no honest row carries one alone; that
    // makes the two reads equivalent for every row the control plane can
    // produce. Taking either as proof is the safer of the two equivalent
    // readings — a row that somehow carried one mark alone would still be one
    // that answered — and it strands nothing: `done` already frees the index
    // whether the answer is there or not, so freezing an answered review holds
    // no later attempt up.
    if task.kind == TaskKind::Review
        && (task.status == TaskStatus::Done || task.review_verdict.is_some())
    {
        return Some(Error::Invalid(format!(
            "review {} already answered {}: a finished review is the record of that \
             verdict, and no status change reopens it",
            task.id,
            task.review_verdict.map_or("nothing", ReviewVerdict::as_str)
        )));
    }
    // An *unanswered* review is finished by its verdict and by nothing else, and
    // `done` is the one status a human could press that would count as
    // finishing it. It
    // would leave a finished review carrying no verdict and no findings, and —
    // because the single-open-review index is written `status NOT IN ('done',
    // 'cancelled', 'dropped', 'released')` — it would free the target for the next review
    // as though this one had answered. That is the whole completion contract
    // walked around, so the domain refuses it and every surface inherits the
    // refusal.
    //
    // Only `done`. `wip` is the reviewer's own path in, taken by a claim.
    // `blocked`, `cancelled` and `dropped` stay pressable *while the review is
    // still open*, because calling an attempt off has to remain possible, and
    // none of them claims an answer; that `cancelled` and `dropped` release the
    // index is the point of abandoning an attempt, not a hole in it.
    if task.kind == TaskKind::Review && to == TaskStatus::Done {
        return Some(Error::Invalid(format!(
            "task {} is a review: it is finished by a verdict \
             (POST /worker/review-report), not by a status change",
            task.id
        )));
    }
    // How a merge attempt *ended* is the worker's answer, never a human's. Both
    // endings the table allows are outcomes carrying evidence that only
    // `report` produces, so a press that names one is a claim about an attempt
    // nobody ran:
    //
    //   * `done` is the landing, and the landing is a transaction, not a
    //     status. `report` reaches it only through `check_gate` — which refuses
    //     a merge that carries no checks or a red one — and then
    //     `land_merge_target`, which moves the target from `approved` to
    //     `merged` against the very commit the merge was issued for. A press
    //     writes neither half. It marks the merge finished while its target
    //     stays `approved`, and that target then falls out of *both*
    //     reconciliation windows at once: `pending_merges` stops at `done`, and
    //     `mergeable` still sees a merge row that is not `cancelled` or
    //     `dropped` holding the target. Approved work, never landed, and
    //     invisible on every screen that exists to notice exactly that.
    //   * `blocked` is the jam, and a jam is a reason. `report_blocked` writes
    //     the worker's account onto `verification` and the red checks onto
    //     `checks_json`; a press writes neither, so it stops the whole
    //     product's train — `claim` hands out nothing behind a blocked merge —
    //     with no reason on the row and no checks under it. The screen shows a
    //     stopped train that cannot say what stopped it.
    //
    // So both are refused here and left to `POST /worker/report`, which is
    // where the evidence comes from.
    //
    // `wip` is deliberately *not* refused, and the line is that it is not an
    // outcome. It says the attempt is running, which is the same thing a claim
    // says; it invents no checks, moves no target, and files no verdict on
    // work nobody did. A merge parked in `wip` by hand holds its train exactly
    // as a claimed one does, and the same two presses — `cancelled` and
    // `dropped` — still release it, so nothing is stranded. Refusing it would
    // reach past the hazard.
    // A release attempt is the same shape: `done` is the tag `bump-tag` cut and
    // the shipping of every task it carries, `blocked` is the reason it could
    // not — both are evidence only `report` produces.
    if matches!(task.kind, TaskKind::InstantMerge | TaskKind::InstantRelease)
        && matches!(to, TaskStatus::Done | TaskStatus::Blocked)
    {
        return Some(Error::Invalid(format!(
            "{} task {} ends the way its worker reports it ended \
             (POST /worker/report, outcome done or blocked), not by a status change: \
             {} pressed by hand would record an attempt that ran with no checks \
             behind it",
            attempt_noun(task.kind),
            task.id,
            to.as_str()
        )));
    }
    // A merge that could not be integrated is a finished attempt, not a paused
    // one, and the way out of it is to call it off and issue a new one — never
    // to restart this row.
    //
    // `blocked -> ready` is the one press the table would still allow, and it
    // is the whole release contract walked around. The row it would hand back
    // to a worker still carries the reason and the checks of the attempt that
    // failed, written by `report_blocked` onto `verification` and
    // `checks_json`; a second attempt on the same row either overwrites that
    // record or — when it blocks for the same reason — is swallowed as the
    // idempotent repeat of a report that described something else entirely.
    // Worse, the merge is pinned to the commit it was issued for, so restarting
    // it re-runs a rebase whose main line has moved on underneath it, and the
    // train it heads starts moving again on evidence that no longer describes
    // anything.
    //
    // `cancelled` and `dropped` stay pressable, because they *are* the release:
    // both are in `MERGE_IS_OVER`, so calling the attempt off frees the target
    // for a new merge, which `mergeable` then offers and `issue_merge` files
    // under a fresh id (`merge:{target}~2`). One human press, one new attempt,
    // and the failed one stays on the record saying why.
    if matches!(task.kind, TaskKind::InstantMerge | TaskKind::InstantRelease)
        && task.status == TaskStatus::Blocked
        && to == TaskStatus::Ready
    {
        return Some(Error::Invalid(format!(
            "{noun} task {} is blocked: a {noun} attempt that could not be integrated is not \
             restarted, it is called off (cancelled or dropped) and issued again against the \
             target as a new attempt",
            task.id,
            noun = attempt_noun(task.kind),
        )));
    }
    None
}

/// The word a refusal uses for an attempt the control plane issued.
fn attempt_noun(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::InstantRelease => "release",
        TaskKind::InstantMerge => "merge",
        TaskKind::Normal | TaskKind::Review => "task",
    }
}

/// Whether the owning product ships releases. A task without a product does not.
fn product_releases(conn: &Connection, product_id: Option<&str>) -> Result<bool, Error> {
    match product_id {
        Some(product_id) => Ok(releases(&product::read(conn, product_id)?)),
        None => Ok(false),
    }
}

/// Whether this product may release *now*.
///
/// The stored flag is derived from the clone's `.github/workflows`, so an
/// archived product — one whose working copy left the project tree — carries the
/// answer of the last walk that could still look. Nothing can check it while the
/// clone is gone, and the release itself needs that clone: the workflows that
/// build the artefacts run from it. So the mark refuses, and it refuses here
/// rather than by rewriting `releases`: the row keeps what the tree last said,
/// and a clone put back releases again on the next walk with nobody re-entering
/// anything.
fn releases(product: &product::Product) -> bool {
    product.releases && !product.archived
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
        Ok(product) if product.archived => Err(Error::Precondition {
            code: "product_archived",
            message: format!(
                "product '{product_id}' is archived: its working copy is not in the \
                 project tree any more, so task {} cannot become ready and nobody \
                 could check it out; restore the clone at {product_id} and restart, \
                 or move the task to a product that is there",
                task.id
            ),
        }),
        Ok(_) => Ok(()),
        // The server does not retain the startup catalogue mode in AppState, so
        // name both remedies. With a derived catalogue a hand-written row is
        // archived by the next walk; with a curated one there is no tree to fix.
        Err(Error::NotFound) => Err(Error::Precondition {
            code: "product_not_catalogued",
            message: format!(
                "product '{product_id}' is not in the product catalogue, \
                 so task {} cannot become ready; correct the product_id, or register \
                 the product through the configured catalogue source: put a clone at \
                 {product_id} and restart when APP_PROJECTS_DIR is set, otherwise use \
                 PUT /api/products/{product_id}",
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
    set_status_pressed_by(db, id, to, now, Presser::ControlPlane)
}

/// Who is asking for a status change. The control plane grants what it owns;
/// an operator is held to [`operator_refusal`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Presser {
    ControlPlane,
    Operator,
}

fn set_status_pressed_by(
    db: &Db,
    id: &str,
    to: TaskStatus,
    now: OffsetDateTime,
    presser: Presser,
) -> Result<Task, Error> {
    let stamp = format_z(now);
    db.with_tx(|tx| {
        let task = read(tx, id)?;
        if presser == Presser::Operator
            && let Some(refusal) = operator_refusal(&task, to)
        {
            return Err(refusal);
        }
        if !can_transition(task.status, to) {
            return Err(Error::Invalid(format!(
                "cannot move task {id} from {} to {}",
                task.status.as_str(),
                to.as_str()
            )));
        }
        if to == TaskStatus::Ready {
            check_catalogued(tx, &task)?;
            // The order is the dependency's to keep, not a person's to skip:
            // clearing `depends_on` is the way past it, and that is deliberate.
            if let Some((dependency, status)) = pending_dependency(tx, &task)? {
                return Err(Error::Precondition {
                    code: "dependency_pending",
                    message: format!(
                        "task {id} depends on {dependency}, which is {} and has not landed; \
                         it becomes ready when that task is merged, or clear depends_on to \
                         skip the order",
                        status.as_str()
                    ),
                });
            }
        }
        if to == TaskStatus::Released && !product_releases(tx, task.product_id.as_deref())? {
            let product_id = task.product_id.as_deref().unwrap_or("<none>");
            return Err(Error::Invalid(format!(
                "product {product_id} does not release"
            )));
        }
        tx.execute(
            "UPDATE tasks SET status = ?2, updated_at = ?3,
                    done_at = CASE WHEN ?2 = 'done' AND kind = 'normal'
                              THEN COALESCE(done_at, ?3) ELSE done_at END
             WHERE id = ?1",
            rusqlite::params![id, to.as_str(), stamp],
        )?;
        // The landing promotes what waited for it; a task called off blocks
        // what waited for it. Both here, so a status the control plane grants
        // directly behaves the same as one a report grants.
        if has_landed(to) {
            promote_dependants(tx, id, &stamp)?;
        }
        if matches!(to, TaskStatus::Cancelled | TaskStatus::Dropped) {
            block_dependants(tx, id, to, &stamp)?;
        }
        read(tx, id)
    })
}

/// Move a task the way a human or an administrative surface moves it.
///
/// `approved`, `merged`, and `released` are earned, not pressed: work is
/// approved by a review that read it, a task lands only when a merge reported
/// green checks, and it ships only through a product release. So is a review's
/// `done`, which belongs to its verdict. Every operator surface — the HTTP
/// status route and the MCP `task_set_status` tool — goes through here, so
/// neither can become a way around the other. The control plane itself keeps
/// calling [`set_status`] directly, because that is how it grants them.
///
/// The refusal is decided inside the same transaction that reads the task, so
/// what is refused is the row as it actually is rather than as it was a moment
/// before.
pub fn set_status_by_operator(
    db: &Db,
    id: &str,
    to: TaskStatus,
    now: OffsetDateTime,
) -> Result<Task, Error> {
    set_status_pressed_by(db, id, to, now, Presser::Operator)
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

/// The merges a claim may take: any whose product is not already held, and any
/// task that is not a merge at all.
///
/// A merge rebases its branch onto the main line, so the merges of one product
/// are serial — the second would otherwise rebase onto a main line the first has
/// not written. A merge is therefore claimable only while no *other* merge of
/// the same product is `wip` or `blocked`: work in flight, and work that stopped
/// and is waiting for a human. `done`, `cancelled` and `dropped` are over and
/// release the rest. **Which of a product's `ready` merges goes first is not
/// decided here, and is not promised anywhere.**
///
/// `IS` rather than `=` on the product, because two merges that carry no product
/// are still each other's train, and `NULL = NULL` would say otherwise.
///
/// Only a merge that is `wip` or `blocked` holds the others up — one that is
/// running, or one that stopped and is waiting for a human. Two `ready` merges
/// do **not** wait on each other: if they did, each would see the other and
/// neither could ever be taken. What keeps them from running together is the
/// claim itself, which takes one row in one transaction; the moment it does,
/// that row is `wip` and the rest of the product's merges wait on it.
///
/// The candidate is excluded from its own test by id, so a merge whose lease
/// expired is still the row that may be taken again rather than the row that
/// blocks itself.
const MERGE_TRAIN_HEAD: &str = "(kind != 'instant:merge'
                                 OR NOT EXISTS (
                                   SELECT 1 FROM tasks ahead
                                   WHERE ahead.kind = 'instant:merge'
                                     AND ahead.id != tasks.id
                                     AND ahead.status IN ('wip', 'blocked')
                                     AND ahead.product_id IS tasks.product_id
                                 ))";

/// Hand the next claimable task to `worker`. The row is only taken while it is
/// still claimable, so no two live leases ever cover the same task. Taking over
/// an expired lease issues a new `claim_id`, which is what invalidates the
/// abandoned one: its holder's report becomes an [`Error::ClaimMismatch`].
pub fn claim(
    db: &Db,
    worker: &str,
    kinds: &[TaskKind],
    now: OffsetDateTime,
    ttl_secs: u64,
) -> Result<Option<Task>, Error> {
    claim_request(db, worker, kinds, None, now, ttl_secs)
}

/// Claim with a retry key that recovers the same live lease after an uncertain
/// response. A key records only a successful claim; `no-work` changed no state
/// and is safe to ask again.
pub fn claim_idempotently(
    db: &Db,
    worker: &str,
    kinds: &[TaskKind],
    idempotency_key: &str,
    now: OffsetDateTime,
    ttl_secs: u64,
) -> Result<Option<Task>, Error> {
    if idempotency_key.trim().is_empty() {
        return Err(Error::Invalid("idempotency_key must not be blank".into()));
    }
    claim_request(db, worker, kinds, Some(idempotency_key), now, ttl_secs)
}

struct ClaimReceipt {
    worker: String,
    kinds: String,
    task_id: String,
    claim_id: String,
}

fn claim_request(
    db: &Db,
    worker: &str,
    kinds: &[TaskKind],
    idempotency_key: Option<&str>,
    now: OffsetDateTime,
    ttl_secs: u64,
) -> Result<Option<Task>, Error> {
    if worker.trim().is_empty() {
        return Err(Error::Invalid("worker is required".into()));
    }
    let kinds_signature = kinds_signature(kinds);
    let ttl = i64::try_from(ttl_secs).unwrap_or(i64::MAX);
    let claimed_at = format_z(now);
    let claim_expires_at = format_z(now + time::Duration::seconds(ttl));
    let select_sql = format!(
        "SELECT {COLUMNS} FROM tasks WHERE {} AND {MERGE_TRAIN_HEAD}{}
         ORDER BY CASE kind WHEN 'instant:merge' THEN 0 ELSE 1 END,
                  priority DESC, created_at ASC, id ASC
         LIMIT 1",
        CLAIMABLE.replace("{now}", "?1"),
        kind_filter(kinds)
    );
    // The guard repeats the candidate predicate exactly; a narrower one would
    // leave an expired lease forever selected and never taken, spinning the loop.
    let update_sql = format!(
        "UPDATE tasks SET status = 'wip', claimed_by = ?2, claim_id = ?3, claimed_at = ?4,
                claim_expires_at = ?5, updated_at = ?4
         WHERE id = ?1 AND {} AND {MERGE_TRAIN_HEAD}",
        CLAIMABLE.replace("{now}", "?4")
    );
    db.with_tx(|tx| {
        if let Some(key) = idempotency_key
            && let Some(receipt) = claim_receipt(tx, key)?
        {
            return replay_claim(tx, key, &receipt, worker, &kinds_signature, &claimed_at)
                .map(Some);
        }
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
                if let Some(key) = idempotency_key {
                    tx.execute(
                        "INSERT INTO claim_receipts
                           (idempotency_key, worker, kinds, task_id, claim_id, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![
                            key,
                            worker,
                            kinds_signature,
                            task.id,
                            claim_id,
                            claimed_at,
                        ],
                    )?;
                }
                return read(tx, &task.id).map(Some);
            }
        }
    })
}

fn claim_receipt(conn: &Connection, key: &str) -> Result<Option<ClaimReceipt>, Error> {
    let mut statement = conn.prepare(
        "SELECT worker, kinds, task_id, claim_id
         FROM claim_receipts WHERE idempotency_key = ?1",
    )?;
    let mut rows = statement.query([key])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(ClaimReceipt {
        worker: row.get(0)?,
        kinds: row.get(1)?,
        task_id: row.get(2)?,
        claim_id: row.get(3)?,
    }))
}

fn replay_claim(
    conn: &Connection,
    key: &str,
    receipt: &ClaimReceipt,
    worker: &str,
    kinds: &str,
    now: &str,
) -> Result<Task, Error> {
    if receipt.worker != worker || receipt.kinds != kinds {
        return Err(claim_idempotency_conflict(format!(
            "idempotency_key {key:?} was already used with another worker or kinds filter"
        )));
    }
    let task = read(conn, &receipt.task_id)?;
    let lease_is_live = task.status == TaskStatus::Wip
        && task.claim_id.as_deref() == Some(receipt.claim_id.as_str())
        && task
            .claim_expires_at
            .as_deref()
            .is_some_and(|expires| expires > now);
    if !lease_is_live {
        return Err(claim_idempotency_conflict(format!(
            "idempotency_key {key:?} no longer names a live lease; retry with a new key"
        )));
    }
    Ok(task)
}

fn claim_idempotency_conflict(message: String) -> Error {
    Error::Precondition {
        code: "claim_idempotency_conflict",
        message,
    }
}

fn kinds_signature(kinds: &[TaskKind]) -> String {
    let mut names: Vec<&str> = kinds.iter().map(|kind| kind.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    names.join(",")
}

/// The `AND kind IN (…)` a claim adds when it asks for particular kinds of work.
///
/// Empty asks for everything, which is what a loop written before the filter
/// sends. The list is spelled by [`TaskKind::as_str`] and never by a caller's
/// text, so it carries no value a bind parameter would have to protect. This is
/// routing, not authorization: a worker that asks for less is given less, and
/// one that asks for everything is still given everything.
fn kind_filter(kinds: &[TaskKind]) -> String {
    if kinds.is_empty() {
        return String::new();
    }
    let list: Vec<&str> = kinds.iter().map(|kind| kind.as_str()).collect();
    format!(" AND kind IN ('{}')", list.join("', '"))
}

/// Hand a live claim back before its lease runs out.
///
/// A worker that is about to go — shutdown, self-update, giving up — would
/// otherwise leave the task `wip` until the lease expires, with nobody able to
/// take it. Handing it back puts the task where the claim found it: `ready`,
/// with the lease columns cleared, so the next claim takes it again. The reason
/// is kept on `verification`, appended to whatever was already there, because a
/// task that came back is a fact the next worker and the operator both read.
///
/// The kind does not matter: a review's `review_attempt` stays as it is (the
/// attempt is over only when a verdict is written), and a merge lets its
/// product's train move on, which is what leaving `wip` does.
///
/// Only a live lease can be handed back. An expired one, one the task already
/// reported on, or an unknown `claim_id` answers with `claim_not_live` — a
/// conflict, because the request is well formed and the world has moved — and
/// writes nothing.
pub fn release_claim(
    db: &Db,
    claim_id: &str,
    reason: &str,
    now: OffsetDateTime,
) -> Result<Task, Error> {
    if claim_id.trim().is_empty() || reason.trim().is_empty() {
        return Err(Error::Invalid("claim_id and reason are required".into()));
    }
    let stamp = format_z(now);
    db.with_tx(|tx| {
        let sql = format!("SELECT {COLUMNS} FROM tasks WHERE claim_id = ?1");
        let Some(task) = query_all(tx, &sql, &[&claim_id])?.pop() else {
            return Err(claim_not_live(
                "no task holds this claim_id; it was already handed back, reported, or retaken",
            ));
        };
        let live = task.status == TaskStatus::Wip
            && task
                .claim_expires_at
                .as_deref()
                .is_some_and(|expires| expires > stamp.as_str());
        if !live {
            return Err(claim_not_live(&format!(
                "task {} is {} and its lease {}; only a live claim can be handed back",
                task.id,
                task.status.as_str(),
                if task.status == TaskStatus::Wip {
                    "has expired"
                } else {
                    "is over"
                }
            )));
        }
        let note = format!(
            "claim released by {}: {}",
            task.claimed_by.as_deref().unwrap_or("<unknown worker>"),
            reason.trim()
        );
        let verification = match task.verification.as_deref() {
            Some(existing) if !existing.trim().is_empty() => format!("{existing}\n{note}"),
            _ => note,
        };
        tx.execute(
            "UPDATE tasks SET status = 'ready', claim_id = NULL, claimed_by = NULL,
                    claimed_at = NULL, claim_expires_at = NULL, verification = ?2,
                    updated_at = ?3
             WHERE id = ?1",
            rusqlite::params![task.id, verification, stamp],
        )?;
        read(tx, &task.id)
    })
}

fn claim_not_live(message: &str) -> Error {
    Error::Precondition {
        code: "claim_not_live",
        message: message.to_owned(),
    }
}

/// Accept a worker's result for the lease `claim_id`.
///
/// For an `instant:merge` task this is the gate onto the main line: the report
/// is only accepted when every check passed, and accepting it lands the target
/// task in the same transaction. A refused report leaves both rows untouched.
///
/// Finishing ordinary work also issues the review that reads it, in this same
/// transaction, because work that is `done` and unread is not a state this
/// control plane has a way out of: the human judgement it asks for is the
/// release, and every step up to it is earned by a report or a verdict. A
/// review that cannot be issued therefore takes the report down with it rather
/// than leaving the work finished and invisible to reviewers.
///
/// `outcome` is how the worker says it could not finish; see [`ReportOutcome`].
/// `release_tag` is read on an `instant:release` task only, where a `done`
/// report is the tag the worker cut.
// One wire body, one function: every field of `/worker/report` arrives here as
// it is, so the two transports cannot disagree about what a report carries.
#[allow(clippy::too_many_arguments)]
pub fn report(
    db: &Db,
    claim_id: &str,
    commit_sha: &str,
    verification: &str,
    checks: &[Check],
    outcome: ReportOutcome,
    release_tag: Option<&str>,
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
        // A review answers with a verdict, and this route has none to record.
        // Accepting it would finish the review without saying anything about
        // its target, and free the one-open-review index on the way out.
        if task.kind == TaskKind::Review {
            return Err(Error::Invalid(format!(
                "task {} is a review: it is finished by a verdict \
                 (POST /worker/review-report), not by a work report",
                task.id
            )));
        }
        if outcome == ReportOutcome::Blocked {
            return report_blocked(tx, &task, verification, checks_json.as_deref(), &stamp);
        }
        // The gate belongs to the report, not to one status: a merge that
        // already landed must still be told the checks passed, or a repeat
        // without evidence would read as "the merge went through with no
        // checks" on the idempotent path. It guards success only — a worker
        // that says it was blocked is *reporting* the red check, not claiming
        // it as a pass.
        if matches!(task.kind, TaskKind::InstantMerge | TaskKind::InstantRelease) {
            check_gate(&task, checks)?;
        }
        // A release is finished by the tag it cut. Nothing else on the report
        // says which version shipped, and a report without one — or with one
        // that is not a version — is refused rather than filed as a release of
        // nothing.
        let release_tag = match task.kind {
            TaskKind::InstantRelease => match release_tag.map(str::trim) {
                Some(tag) if is_release_tag(tag) => Some(tag),
                _ => {
                    return Err(Error::Invalid(format!(
                        "release task {} is reported with the tag it cut: release_tag \
                         must match v<major>.<minor>.<patch>",
                        task.id
                    )));
                }
            },
            _ => None,
        };
        match task.status {
            TaskStatus::Wip => {
                tx.execute(
                    "UPDATE tasks SET status = 'done', commit_sha = ?2, verification = ?3,
                            checks_json = ?4, updated_at = ?5,
                            done_at = CASE WHEN kind = 'normal'
                                      THEN COALESCE(done_at, ?5) ELSE done_at END
                     WHERE id = ?1",
                    rusqlite::params![task.id, commit_sha, verification, checks_json, stamp],
                )?;
                match task.kind {
                    TaskKind::InstantMerge => land_merge_target(tx, &task, &stamp)?,
                    TaskKind::Normal => ensure_review(tx, &task.id, &stamp)?,
                    TaskKind::InstantRelease => {
                        let tag = release_tag.unwrap_or_default();
                        ship_release(tx, &task, tag, &stamp)?;
                    }
                    TaskKind::Review => unreachable!("a review is refused above"),
                }
                read(tx, &task.id)
            }
            // The repeat of a report already on the record finishes nothing a
            // second time, so it issues nothing either: the review the first
            // one filed is still the review of this commit.
            TaskStatus::Done if task.commit_sha.as_deref() == Some(commit_sha) => Ok(task),
            // A release that shipped left `done` for `released` in the same
            // write, so its repeat arrives at a released row.
            TaskStatus::Released
                if task.kind == TaskKind::InstantRelease
                    && task.commit_sha.as_deref() == Some(commit_sha) =>
            {
                Ok(task)
            }
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

/// Write down that a worker could not finish, and stop there.
///
/// This is the one report that commits a failure instead of rolling it back.
/// A merge that hit a rebase conflict, or a check that came back red, has to
/// leave a record a human can read — the alternative is the merge sitting in
/// `wip` until its lease expires, handed straight back to the next worker to
/// fail the same way, with nothing anywhere saying why.
///
/// The target of a blocked merge is deliberately left where it was: nothing
/// landed, so nothing moves. `commit_sha` is not overwritten either — on a merge
/// it is the subject the merge was issued for, and a later attempt is checked
/// against it.
fn report_blocked(
    tx: &Connection,
    task: &Task,
    reason: &str,
    checks_json: Option<&str>,
    stamp: &str,
) -> Result<Task, Error> {
    match task.status {
        TaskStatus::Wip => {
            tx.execute(
                "UPDATE tasks SET status = 'blocked', verification = ?2, checks_json = ?3,
                        updated_at = ?4
                 WHERE id = ?1",
                rusqlite::params![task.id, reason, checks_json, stamp],
            )?;
            read(tx, &task.id)
        }
        // A worker that did not hear the answer sends the same report again.
        TaskStatus::Blocked if task.verification.as_deref() == Some(reason) => Ok(task.clone()),
        TaskStatus::Blocked => Err(Error::Invalid(format!(
            "task {} is already blocked for another reason: {}",
            task.id,
            task.verification.as_deref().unwrap_or("<none>")
        ))),
        other => Err(Error::Invalid(format!(
            "task {} cannot be blocked from {}",
            task.id,
            other.as_str()
        ))),
    }
}

/// See to it that `target_id` has a review reading it, issuing one if it has
/// none.
///
/// A review that is already open is already the answer to "who is reading this",
/// even when the work has moved on to another commit since — that review either
/// hands the work back or is refused as stale by `review_subject_changed`, and
/// either way the reader exists. Issuing a second one is impossible anyway: the
/// single-open-review index would refuse it, and refusing it here would take the
/// worker's report down with it for no gain.
fn ensure_review(tx: &Connection, target_id: &str, stamp: &str) -> Result<(), Error> {
    if has_open_attempt(tx, "review_target_task_id", REVIEW_IS_OVER, target_id)? {
        return Ok(());
    }
    issue_review_in_tx(tx, target_id, stamp)?;
    Ok(())
}

/// See to it that `target_id` has a merge landing it, issuing one if it has
/// none. The counterpart of [`ensure_review`], on the other side of a verdict.
fn ensure_merge(tx: &Connection, target_id: &str, stamp: &str) -> Result<(), Error> {
    if has_open_attempt(tx, "merge_target_task_id", MERGE_IS_OVER, target_id)? {
        return Ok(());
    }
    issue_merge_in_tx(tx, target_id, stamp)?;
    Ok(())
}

/// The statuses that end a review's hold on its target, spelled as the partial
/// unique index spells it. A review is over the moment it answers, and a review
/// carried to `released` with its shipped target is over as well — two of them
/// may share a target, because a target reviewed twice ships both its rounds.
///
/// One phrase, three readers: [`ensure_review`] asks it before issuing,
/// [`pending_reviews`] lists the attempts it excludes, and [`unreviewed`]
/// reports the targets no attempt holds. They have to agree — a target is
/// listed as unreviewed exactly when a new review could be issued for it — so
/// they read it from here instead of each spelling it out.
///
/// The migration that (re)creates the index spells it again in its own SQL, and
/// stays there on purpose: a migration is the record of what a past schema was
/// made of, and it must not change under a later edit to this line. Version 5
/// stopped at `done`; version 12 added `released`.
const REVIEW_IS_OVER: &str = "('done', 'cancelled', 'dropped', 'released')";

/// The statuses that end a merge's hold on its target. A landed merge keeps its
/// target for ever — `done` is not on this list — because a task that merged is
/// not an invitation to merge it again.
///
/// Read by [`ensure_merge`] before issuing and by [`mergeable`] when it offers
/// a target to a human, for the same reason [`REVIEW_IS_OVER`] is shared: what
/// a screen offers to merge is exactly what `issue_merge` would accept.
///
/// Not to be confused with the end of [`pending_merges`], which also stops at
/// `done`. That list is "attempts still in flight", and a landed merge is no
/// longer in flight even though it still holds its target for ever. Two
/// different questions, and this constant answers only the second.
const MERGE_IS_OVER: &str = "('cancelled', 'dropped')";

/// Whether some attempt still holds `target_id` through `column`.
///
/// Asked with the predicate of the index that forbids a second one, so this
/// answers `true` exactly when an issue would be refused as a conflict.
fn has_open_attempt(
    conn: &Connection,
    column: &str,
    is_over: &str,
    target_id: &str,
) -> Result<bool, Error> {
    Ok(conn.query_row(
        &format!(
            "SELECT EXISTS(
               SELECT 1 FROM tasks WHERE {column} = ?1 AND status NOT IN {is_over}
             )"
        ),
        [target_id],
        |row| row.get(0),
    )?)
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

/// Move the task a finished merge landed from `approved` to `merged`.
///
/// `merge` is the merge row **as the report found it**, read before the report
/// writes its own commit over it, so `merge.commit_sha` is still the commit the
/// merge was issued for. That snapshot is what this checks against; re-reading
/// the row here would answer with the merge commit the worker just reported and
/// check nothing.
///
/// Both questions — is the target still waiting, and is it still standing on
/// the commit this merge was issued for — are asked inside the transaction that
/// would land it, and a refusal writes nothing at all.
fn land_merge_target(tx: &Connection, merge: &Task, stamp: &str) -> Result<(), Error> {
    let Some(target_id) = merge.merge_target_task_id.as_deref() else {
        return Err(Error::Invalid(format!(
            "merge task {} has no target to land",
            merge.id
        )));
    };
    let target = read(tx, target_id)?;
    if target.status != TaskStatus::Approved {
        return Err(Error::Invalid(format!(
            "task {target_id} is {}, so merge task {} cannot land it",
            target.status.as_str(),
            merge.id
        )));
    }
    // The review side of this guard, on the other end of the same hazard: an
    // approval of a commit the work has left behind is refused there, and a
    // merge of one is refused here. `approved` alone does not say *which*
    // commit was approved, so a merge issued for commit A can arrive after the
    // work was reopened, redone as B, reviewed and approved again — and landing
    // it would put A on the main line under B's approval, or mark the task
    // merged when what actually merged was the commit nobody approved.
    if target.commit_sha != merge.commit_sha {
        return Err(Error::Precondition {
            code: "merge_subject_changed",
            message: format!(
                "task {target_id} is on commit {}, and merge task {} was issued for {}; \
                 issue a merge for the new commit instead of landing the old one",
                target.commit_sha.as_deref().unwrap_or("<no commit>"),
                merge.id,
                merge.commit_sha.as_deref().unwrap_or("<no commit>")
            ),
        });
    }
    tx.execute(
        "UPDATE tasks SET status = 'merged', updated_at = ?2 WHERE id = ?1",
        rusqlite::params![target_id, stamp],
    )?;
    promote_dependants(tx, target_id, stamp)?;
    // Landing is where the release is decided. A product that ships has its
    // release issued here, in the same transaction, the way a report issues
    // its review; one that does not ship is done the moment it lands.
    let Some(product_id) = target.product_id.as_deref() else {
        return Ok(());
    };
    match product::read(tx, product_id) {
        Ok(product) if releases(&product) => {
            ensure_release(tx, product_id, stamp)?;
        }
        Ok(product) if !product.releases => release_without_tag(tx, target_id, stamp)?,
        // Archived: the flag cannot be re-read and the workflows cannot run.
        // The work stays merged, and `releasable` shows it as stranded.
        Ok(_) | Err(Error::NotFound) => {}
        Err(other) => return Err(other),
    }
    Ok(())
}

/// The statuses in which a release attempt still holds its product: issued and
/// waiting, running, or stopped and waiting for a person. A product carries at
/// most one of these at a time, and while one is there no second release is
/// issued — the report that ends it calls [`ensure_release`] again and picks
/// up whatever merged in the meantime.
const RELEASE_IS_OPEN: &str = "('ready', 'wip', 'blocked')";

/// The `normal` work of a product that has landed and has no live release
/// carrying it: never issued one, or issued one that was called off. What
/// [`ensure_release`] gathers, what `POST /api/releases` gathers by hand, and
/// what [`releasable`] reports as stranded — one predicate, three readers.
///
/// Newest first, so the first row names the release: `release:<newest target>`.
const RELEASE_TARGETS: &str = "kind = 'normal' AND status = 'merged' AND product_id = ?1
                               AND (release_task_id IS NULL OR NOT EXISTS (
                                 SELECT 1 FROM tasks carrier
                                 WHERE carrier.id = tasks.release_task_id
                                   AND carrier.status NOT IN ('cancelled', 'dropped')
                               ))";

/// Whether some release attempt of `product_id` is still open.
fn has_open_release(conn: &Connection, product_id: &str) -> Result<bool, Error> {
    Ok(conn.query_row(
        &format!(
            "SELECT EXISTS(
               SELECT 1 FROM tasks
               WHERE kind = 'instant:release' AND product_id = ?1
                 AND status IN {RELEASE_IS_OPEN}
             )"
        ),
        [product_id],
        |row| row.get(0),
    )?)
}

fn release_targets(conn: &Connection, product_id: &str) -> Result<Vec<Task>, Error> {
    query_all(
        conn,
        &format!(
            "SELECT {COLUMNS} FROM tasks
             WHERE {RELEASE_TARGETS}
             ORDER BY created_at DESC, id DESC"
        ),
        &[&product_id],
    )
}

/// See to it that the landed work of `product_id` has a release carrying it,
/// issuing one if it has none and there is something to ship.
///
/// Skips while a release is open: that attempt either ships, and its report
/// comes back here, or it is called off, and the next merge or a hand-issued
/// release gathers everything again. Skips too when nothing merged is waiting.
fn ensure_release(tx: &Connection, product_id: &str, stamp: &str) -> Result<Option<Task>, Error> {
    if has_open_release(tx, product_id)? {
        return Ok(None);
    }
    let targets = release_targets(tx, product_id)?;
    if targets.is_empty() {
        return Ok(None);
    }
    issue_release_for(tx, product_id, &targets, stamp).map(Some)
}

/// Write the release task that ships `targets` and point each of them at it.
///
/// The level is the largest level among the work shipped, the id is derived
/// from the newest target, and the priority is the highest the work carried, so
/// a release is never handed out behind the work that earned it.
fn issue_release_for(
    tx: &Connection,
    product_id: &str,
    targets: &[Task],
    stamp: &str,
) -> Result<Task, Error> {
    let newest = targets.first().ok_or_else(|| {
        Error::Conflict(format!(
            "product {product_id} has no merged task to release"
        ))
    })?;
    let level = targets
        .iter()
        .map(|task| task.release_level)
        .max()
        .unwrap_or_default();
    let priority = targets.iter().map(|task| task.priority).max().unwrap_or(0);
    let (id, _attempt) = free_attempt_id(tx, &release_task_id(&newest.id))?;
    tx.execute(
        "INSERT INTO tasks (id, title, body, status, kind, product_id, priority, release_level,
                            created_at, updated_at)
         VALUES (?1, ?2, '', 'ready', 'instant:release', ?3, ?4, ?5, ?6, ?6)",
        rusqlite::params![
            id,
            format!("release {product_id}: {}", level.as_str()),
            product_id,
            priority,
            level.as_str(),
            stamp,
        ],
    )?;
    for target in targets {
        tx.execute(
            "UPDATE tasks SET release_task_id = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![target.id, id, stamp],
        )?;
    }
    read(tx, &id)
}

/// Issue the release of `product_id` by hand: the reconciliation handle for a
/// product whose landed work lost its release — an attempt that was called off
/// — or that merged before releases were issued automatically.
///
/// A product that does not ship, one with an attempt still open, and one with
/// nothing waiting each answer with a conflict; the request is well formed and
/// the world refuses it.
pub fn issue_release(db: &Db, product_id: &str, now: OffsetDateTime) -> Result<Task, Error> {
    let stamp = format_z(now);
    db.with_tx(|tx| {
        let product = product::read(tx, product_id)?;
        if !releases(&product) {
            return Err(Error::Conflict(format!(
                "product {product_id} does not release"
            )));
        }
        if has_open_release(tx, product_id)? {
            return Err(Error::Conflict(format!(
                "product {product_id} already has a release in flight"
            )));
        }
        let targets = release_targets(tx, product_id)?;
        if targets.is_empty() {
            return Err(Error::Conflict(format!(
                "product {product_id} has no merged task to release"
            )));
        }
        issue_release_for(tx, product_id, &targets, &stamp)
    })
}

/// Ship what `release` carries under `tag`: the release task itself, every
/// `normal` task pointing at it, and the finished `review` and `instant:merge`
/// subtasks of those — all to `released`, one tag, one transaction. Then look
/// again: work that landed while this release ran gets the next one.
fn ship_release(tx: &Connection, release: &Task, tag: &str, stamp: &str) -> Result<(), Error> {
    tx.execute(
        "UPDATE tasks SET status = 'released', release_tag = ?2, updated_at = ?3
         WHERE kind = 'normal' AND status = 'merged' AND release_task_id = ?1",
        rusqlite::params![release.id, tag, stamp],
    )?;
    release_subtasks_of(
        tx,
        "release_task_id = ?1",
        &[&release.id, &Some(tag), &stamp],
    )?;
    tx.execute(
        "UPDATE tasks SET status = 'released', release_tag = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![release.id, tag, stamp],
    )?;
    if let Some(product_id) = release.product_id.as_deref() {
        ensure_release(tx, product_id, stamp)?;
    }
    Ok(())
}

/// A product that does not ship ends at the landing: the work and its finished
/// subtasks go straight to `released`, with no tag, because there is none.
fn release_without_tag(tx: &Connection, target_id: &str, stamp: &str) -> Result<(), Error> {
    tx.execute(
        "UPDATE tasks SET status = 'released', updated_at = ?2 WHERE id = ?1",
        rusqlite::params![target_id, stamp],
    )?;
    release_subtasks_of(tx, "id = ?1", &[&target_id, &None::<&str>, &stamp])
}

/// Carry the finished `review` and `instant:merge` subtasks of the `normal`
/// tasks selected by `targets_where` (bound to `?1`) to `released` under `?2`,
/// stamped `?3`. A subtask left at `done` after its target shipped is a husk:
/// the verdict lives on the target's `latest_review` and the merge landed.
fn release_subtasks_of(
    tx: &Connection,
    targets_where: &str,
    params: &[&dyn ToSql],
) -> Result<(), Error> {
    tx.execute(
        &format!(
            "UPDATE tasks SET status = 'released', release_tag = ?2, updated_at = ?3
             WHERE status = 'done' AND kind IN ('review', 'instant:merge')
               AND COALESCE(review_target_task_id, merge_target_task_id) IN (
                 SELECT id FROM tasks WHERE kind = 'normal' AND status = 'released'
                   AND {targets_where}
               )"
        ),
        params,
    )?;
    Ok(())
}

/// Release tasks that have been issued and not finished yet, oldest first: what
/// a screen shows as "shipping", with the reason on a stopped one.
pub fn pending_releases(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks
                 WHERE kind = 'instant:release'
                   AND status IN {RELEASE_IS_OPEN}
                 ORDER BY created_at ASC, id ASC"
            ),
            &[],
        )
    })
}

/// The id of the first release task shipping `newest_target_id`.
#[must_use]
pub fn release_task_id(newest_target_id: &str) -> String {
    format!("release:{newest_target_id}")
}

/// The tasks a human may press "merge" on: approved normal work that carries
/// the branch and commit a worker needs, and that no live merge already owns.
///
/// `done` is not enough. Work reaches this list only after a review read the
/// commit and approved it, so nothing goes onto the main line unread.
pub fn mergeable(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks
                 WHERE kind = 'normal' AND status = 'approved'
                   AND branch IS NOT NULL AND commit_sha IS NOT NULL
                   AND NOT EXISTS (
                     SELECT 1 FROM tasks live
                     WHERE live.merge_target_task_id = tasks.id
                       AND live.status NOT IN {MERGE_IS_OVER}
                   )
                 ORDER BY created_at ASC, id ASC"
            ),
            &[],
        )
    })
}

/// Merge tasks that have been issued and not finished yet.
///
/// The order is stable — oldest first, ties broken by id — and it is only that.
/// Which merge is handed out next is not promised: any of a product's `ready`
/// merges may be the one a claim takes. A screen showing this list is showing
/// what is outstanding, not a queue position.
pub fn pending_merges(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks
                 WHERE kind = 'instant:merge'
                   AND status NOT IN ('done', 'cancelled', 'dropped', 'released')
                 ORDER BY created_at ASC, id ASC"
            ),
            &[],
        )
    })
}

/// Review tasks that have been issued and not answered yet. The mirror of
/// [`pending_merges`], and what a screen shows as "waiting to be read".
pub fn pending_reviews(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks
                 WHERE kind = 'review'
                   AND status NOT IN {REVIEW_IS_OVER}
                 ORDER BY created_at ASC, id ASC"
            ),
            &[],
        )
    })
}

/// Work that is `done` with no live review reading it.
///
/// This is an alarm, not a queue. A `done` report issues its own review in the
/// same transaction, so in a healthy control plane this list is empty and stays
/// empty; anything in it is work that finished and then lost its reader — an
/// attempt somebody cancelled, or a row from before the issuing was automatic —
/// and it will sit there for ever, because `done` has no way forward except a
/// verdict. Nobody is meant to act on it as a matter of course. Its job is to
/// make that silence visible instead of leaving the work quietly stranded.
///
/// "Live" is spelled exactly as the single-open-review index spells it, so a
/// task is listed here precisely when a new review could be issued for it.
pub fn unreviewed(db: &Db) -> Result<Vec<Task>, Error> {
    db.with_conn(|conn| {
        query_all(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM tasks
                 WHERE kind = 'normal' AND status = 'done'
                   AND NOT EXISTS (
                     SELECT 1 FROM tasks live
                     WHERE live.review_target_task_id = tasks.id
                       AND live.status NOT IN {REVIEW_IS_OVER}
                   )
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

/// The id of the first review task that reads `target_id`. Derived, so a target
/// and its review are readable as a pair.
#[must_use]
pub fn review_task_id(target_id: &str) -> String {
    format!("review:{target_id}")
}

/// The `attempt`-th id derived from `base`. The first attempt keeps the plain
/// id; a retry appends `~2`, `~3`, … `~` is unreserved in a URI, so the id stays
/// one path segment.
fn attempt_id(base: &str, attempt: u32) -> String {
    match attempt {
        1 => base.to_owned(),
        n => format!("{base}~{n}"),
    }
}

/// The first id derived from `base` that no row has taken yet, with the attempt
/// number it stands for.
///
/// The partial unique index, not this walk, is what forbids a second *live*
/// merge or review; this only keeps a permitted retry from colliding with the
/// primary key of an attempt that is already on the record. It runs inside the
/// caller's transaction, so a racing issue serializes behind it and then loses
/// on the index instead of stealing the id.
///
/// The number is returned rather than left to be read back out of the id: `~9`
/// and `~10` compare the wrong way round as text, so whoever needs the order of
/// two attempts needs the integer.
fn free_attempt_id(conn: &Connection, base: &str) -> Result<(String, u32), Error> {
    let mut attempt = 1;
    loop {
        let id = attempt_id(base, attempt);
        let taken: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
            [&id],
            |row| row.get(0),
        )?;
        if !taken {
            return Ok((id, attempt));
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
    db.with_tx(|tx| issue_merge_in_tx(tx, target_id, &stamp))
}

/// Issue the merge inside a transaction the caller already owns.
///
/// The manual route opens its own transaction around this; an approving verdict
/// calls it inside the transaction that granted the approval, so the promotion
/// and the merge it earns are one write. One body, so the two ways in cannot
/// drift apart on what a merge inherits or on what it refuses.
fn issue_merge_in_tx(tx: &Connection, target_id: &str, stamp: &str) -> Result<Task, Error> {
    let target = read(tx, target_id)?;
    if target.kind != TaskKind::Normal {
        return Err(Error::Invalid(format!(
            "task {target_id} is {}, and only normal work is merged",
            target.kind.as_str()
        )));
    }
    if target.status != TaskStatus::Approved {
        return Err(Error::Invalid(format!(
            "task {target_id} is {}, so it is not ready to merge: only work a review \
             approved is merged",
            target.status.as_str()
        )));
    }
    let (Some(branch), Some(commit_sha)) = (&target.branch, &target.commit_sha) else {
        return Err(Error::Invalid(format!(
            "task {target_id} has no branch and commit to merge"
        )));
    };
    let (id, _attempt) = free_attempt_id(tx, &merge_task_id(target_id))?;
    tx.execute(
        "INSERT INTO tasks (id, title, body, status, kind, product_id, priority, branch,
                            commit_sha, merge_target_task_id, release_level,
                            created_at, updated_at)
         VALUES (?1, ?2, '', 'ready', 'instant:merge', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        rusqlite::params![
            id,
            format!("merge {target_id}: {}", target.title),
            target.product_id,
            target.priority,
            branch,
            commit_sha,
            target_id,
            target.release_level.as_str(),
            stamp,
        ],
    )
    .map_err(|err| {
        attempt_conflict(
            err,
            format!("task {target_id} already has a merge in flight"),
        )
    })?;
    read(tx, &id)
}

/// The partial unique index (and the primary key) is what actually forbids a
/// second live attempt; a constraint violation here is a conflict, not a bug.
fn attempt_conflict(err: rusqlite::Error, message: String) -> Error {
    match err {
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Error::Conflict(message)
        }
        other => other.into(),
    }
}

/// Issue the `review` task that reads `target_id`.
///
/// The review inherits the target's product, branch and priority, and takes a
/// snapshot of the commit the work reported: that snapshot is the subject of the
/// review, and the approval that arrives later is only accepted for it. The
/// review's own completion never rewrites it.
pub fn issue_review(db: &Db, target_id: &str, now: OffsetDateTime) -> Result<Task, Error> {
    let stamp = format_z(now);
    db.with_tx(|tx| issue_review_in_tx(tx, target_id, &stamp))
}

/// Issue the review inside a transaction the caller already owns.
///
/// The manual route opens its own transaction around this; a `done` report calls
/// it inside the transaction that finished the work, so finishing and being read
/// are one write. One body, so neither way in can drift from the other.
fn issue_review_in_tx(tx: &Connection, target_id: &str, stamp: &str) -> Result<Task, Error> {
    let target = read(tx, target_id)?;
    if target.kind != TaskKind::Normal {
        return Err(Error::Invalid(format!(
            "task {target_id} is {}, and only normal work is reviewed",
            target.kind.as_str()
        )));
    }
    if target.status != TaskStatus::Done {
        return Err(Error::Invalid(format!(
            "task {target_id} is {}, so there is nothing to review yet",
            target.status.as_str()
        )));
    }
    let (Some(branch), Some(commit_sha)) = (&target.branch, &target.commit_sha) else {
        return Err(Error::Invalid(format!(
            "task {target_id} has no branch and commit to review"
        )));
    };
    let (id, attempt) = free_attempt_id(tx, &review_task_id(target_id))?;
    tx.execute(
        "INSERT INTO tasks (id, title, body, status, kind, product_id, priority, branch,
                            commit_sha, review_target_task_id, review_attempt, release_level,
                            created_at, updated_at)
         VALUES (?1, ?2, '', 'ready', 'review', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
        rusqlite::params![
            id,
            format!("review {target_id}: {}", target.title),
            target.product_id,
            target.priority,
            branch,
            commit_sha,
            target_id,
            attempt,
            target.release_level.as_str(),
            stamp,
        ],
    )
    .map_err(|err| {
        attempt_conflict(
            err,
            format!(
                "task {target_id} already has a review in flight; cancel it before the work \
                 is finished again"
            ),
        )
    })?;
    read(tx, &id)
}

/// Accept a reviewer's verdict for the lease `claim_id`.
///
/// This is the review's completion contract, and deliberately not
/// [`report`]: `request_changes` is a finished review rather than a failed one,
/// so it is a success, and there are no checks to gate on — a reviewer's
/// evidence is what they wrote, not a process exit code.
///
/// An approval is only accepted for the commit the review was issued for. All
/// three questions — is the parent still waiting, is it still on that commit,
/// and did the reviewer name that commit — are asked inside the one transaction
/// that would promote it, so an approval can never overtake a change made while
/// the review was open. A refusal writes nothing at all.
pub fn review_report(
    db: &Db,
    claim_id: &str,
    subject_commit_sha: &str,
    verdict: ReviewVerdict,
    findings: &str,
    now: OffsetDateTime,
) -> Result<Task, Error> {
    if subject_commit_sha.trim().is_empty() || findings.trim().is_empty() {
        return Err(Error::Invalid(
            "subject_commit_sha and findings are required".into(),
        ));
    }
    let stamp = format_z(now);
    db.with_tx(|tx| {
        let sql = format!("SELECT {COLUMNS} FROM tasks WHERE claim_id = ?1");
        let Some(review) = query_all(tx, &sql, &[&claim_id])?.pop() else {
            return Err(Error::ClaimMismatch);
        };
        if review.kind != TaskKind::Review {
            return Err(Error::Invalid(format!(
                "task {} is {}, and a verdict finishes a review only",
                review.id,
                review.kind.as_str()
            )));
        }
        let Some(target_id) = review.review_target_task_id.clone() else {
            return Err(Error::Invalid(format!(
                "review task {} has no target to answer for",
                review.id
            )));
        };
        match review.status {
            TaskStatus::Wip => {
                answer_review(tx, &review, &target_id, verdict, subject_commit_sha)?;
                tx.execute(
                    "UPDATE tasks SET status = 'done', review_verdict = ?2, verification = ?3,
                            updated_at = ?4
                     WHERE id = ?1",
                    rusqlite::params![review.id, verdict.as_str(), findings, stamp],
                )?;
                tx.execute(
                    "UPDATE tasks SET status = ?2, updated_at = ?3 WHERE id = ?1",
                    rusqlite::params![target_id, verdict_moves_target_to(verdict), stamp],
                )?;
                // An approval is the last judgement before the main line, so
                // the merge it earns is issued here rather than waited for: a
                // human pressing "merge" afterwards would be pressing a button
                // whose answer the review already gave.
                if verdict == ReviewVerdict::Approve {
                    ensure_merge(tx, &target_id, &stamp)?;
                }
                read(tx, &review.id)
            }
            // A repeat of the answer already on the record is accepted and
            // moves nothing: the target left `done` the moment the first one
            // landed, so re-applying it would be refused for the wrong reason.
            TaskStatus::Done
                if review.review_verdict == Some(verdict)
                    && review.commit_sha.as_deref() == Some(subject_commit_sha) =>
            {
                Ok(review)
            }
            TaskStatus::Done => Err(Error::Invalid(format!(
                "review {} already answered {} for {}",
                review.id,
                review
                    .review_verdict
                    .map_or("nothing", ReviewVerdict::as_str),
                review.commit_sha.as_deref().unwrap_or("<no commit>")
            ))),
            other => Err(Error::Invalid(format!(
                "review {} cannot be reported from {}",
                review.id,
                other.as_str()
            ))),
        }
    })
}

/// Where a verdict leaves the reviewed task.
///
/// `request_changes` is the one way back out of `done`, and it deliberately
/// skips the catalogue gate that guards an ordinary promotion to `ready`: this
/// is the continuation of work already admitted, not a new admission, and a
/// product whose clone left the tree would otherwise leave the task hanging —
/// impossible to approve and impossible to hand back.
fn verdict_moves_target_to(verdict: ReviewVerdict) -> &'static str {
    match verdict {
        ReviewVerdict::Approve => TaskStatus::Approved.as_str(),
        ReviewVerdict::RequestChanges => TaskStatus::Ready.as_str(),
    }
}

/// Whether the world is still the one this review was issued for. Each refusal
/// carries its own code, because the remedies differ: report the right commit,
/// wait for the task to come back to `done`, or review the new commit.
fn answer_review(
    tx: &Connection,
    review: &Task,
    target_id: &str,
    verdict: ReviewVerdict,
    subject_commit_sha: &str,
) -> Result<(), Error> {
    if review.commit_sha.as_deref() != Some(subject_commit_sha) {
        return Err(Error::Precondition {
            code: "review_subject_mismatch",
            message: format!(
                "review {} is of commit {}, not {subject_commit_sha}",
                review.id,
                review.commit_sha.as_deref().unwrap_or("<no commit>")
            ),
        });
    }
    let target = read(tx, target_id)?;
    if target.status != TaskStatus::Done {
        return Err(Error::Precondition {
            code: "review_target_moved",
            message: format!(
                "task {target_id} is {}, so review {} has nothing to answer for; \
                 it is answered while the work waits in done",
                target.status.as_str(),
                review.id
            ),
        });
    }
    // Only an approval is stopped by a commit that moved on. Handing work back
    // is safe whatever it is standing on; carrying it forward is not, because
    // the commit that would land is one nobody read.
    if verdict == ReviewVerdict::Approve && target.commit_sha != review.commit_sha {
        return Err(Error::Precondition {
            code: "review_subject_changed",
            message: format!(
                "task {target_id} is on commit {}, and review {} read {}; \
                 review the new commit instead of approving the old one",
                target.commit_sha.as_deref().unwrap_or("<no commit>"),
                review.id,
                review.commit_sha.as_deref().unwrap_or("<no commit>")
            ),
        });
    }
    Ok(())
}

/// What the latest finished review of `target_id` said, if one has.
///
/// Read from the review task's own row rather than copied onto the target, so a
/// worker sent back to the queue reads the findings from the one place they were
/// written. An open review has no verdict yet and does not answer here.
///
/// "Latest" is the highest attempt number, and nothing else. The attempt is the
/// authority because attempts of one target are strictly serial: the
/// single-open-review index means attempt *n+1* cannot be issued until attempt
/// *n* is out of flight, and a review only ever answers from `wip`, so a higher
/// attempt is always the later answer. Timestamps could not decide it —
/// [`format_z`] writes whole seconds, so two attempts finished in the same
/// second tie — and the derived ids could not either, because `review:t~9`
/// sorts after `review:t~10` as text.
pub fn latest_review(db: &Db, target_id: &str) -> Result<Option<ReviewOutcome>, Error> {
    db.with_conn(|conn| latest_review_of(conn, target_id))
}

fn latest_review_of(conn: &Connection, target_id: &str) -> Result<Option<ReviewOutcome>, Error> {
    let sql = format!(
        "SELECT {COLUMNS} FROM tasks
         WHERE review_target_task_id = ?1 AND review_verdict IS NOT NULL
         ORDER BY review_attempt DESC
         LIMIT 1"
    );
    let Some(review) = query_all(conn, &sql, &[&target_id])?.pop() else {
        return Ok(None);
    };
    let Some(verdict) = review.review_verdict else {
        return Ok(None);
    };
    Ok(Some(ReviewOutcome {
        review_task_id: review.id,
        verdict,
        findings: review.verification,
        subject_commit_sha: review.commit_sha,
        reported_at: review.updated_at,
    }))
}

/// Landed work with no release carrying it, per releasing product.
///
/// Work that lands while a release of its product is still open is not
/// stranded — the report that ends that release gathers it — so a product with
/// an open release is left out.
///
/// This is an alarm, not a queue. A landing issues its own release, so in a
/// healthy control plane this list is empty; anything in it is work whose
/// release was called off, or that merged before releases were issued
/// automatically, and `POST /api/releases` is the handle that clears it. An
/// archived product is left out for the reason [`releases`] gives: it could not
/// be released if it were asked, so offering it would be an invitation to a
/// refusal.
pub fn releasable(db: &Db) -> Result<Vec<Releasable>, Error> {
    db.with_conn(|conn| {
        let mut statement = conn.prepare(&format!(
            "SELECT tasks.product_id, count(*) FROM tasks
             JOIN products ON products.id = tasks.product_id
             WHERE products.releases = 1 AND products.archived_at IS NULL
               AND tasks.kind = 'normal' AND tasks.status = 'merged'
               AND (tasks.release_task_id IS NULL OR NOT EXISTS (
                 SELECT 1 FROM tasks carrier
                 WHERE carrier.id = tasks.release_task_id
                   AND carrier.status NOT IN ('cancelled', 'dropped')
               ))
               AND NOT EXISTS (
                 SELECT 1 FROM tasks open
                 WHERE open.kind = 'instant:release' AND open.product_id = tasks.product_id
                   AND open.status IN {RELEASE_IS_OPEN}
               )
             GROUP BY tasks.product_id
             ORDER BY tasks.product_id ASC"
        ))?;
        let rows = statement.query_map([], |row| {
            Ok(Releasable {
                product_id: row.get(0)?,
                task_count: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
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
    // No `done -> ready` sits here on purpose. Work comes back from `done` only
    // through a review that requested changes, which has its own operation; a
    // generic edge would let anyone reopen finished work by hand.
    matches!(
        (from, to),
        (TaskStatus::Draft | TaskStatus::Blocked, TaskStatus::Ready)
            | (TaskStatus::Ready, TaskStatus::Wip)
            | (TaskStatus::Wip, TaskStatus::Done | TaskStatus::Ready)
            | (TaskStatus::Done, TaskStatus::Approved)
            | (TaskStatus::Approved, TaskStatus::Merged)
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
        review_target_task_id: row.get(19)?,
        review_verdict: decode_verdict(row.get::<_, Option<String>>(20)?.as_deref())?,
        release_level: ReleaseLevel::parse(&row.get::<_, String>(21)?)?,
        release_task_id: row.get(22)?,
        depends_on: row.get(23)?,
        done_at: row.get(24)?,
    })
}

fn decode_verdict(raw: Option<&str>) -> Result<Option<ReviewVerdict>, Error> {
    raw.map(ReviewVerdict::parse).transpose()
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
        ALL_STATUSES, Check, NewTask, Releasable, ReleaseLevel, ReportOutcome, ReviewVerdict, Task,
        TaskKind, TaskPatch, TaskStatus, available_transitions, can_transition, claim, create,
        dependency_status, get, issue_merge, issue_release, issue_review, latest_review, list,
        list_active, list_by_status, list_done, merge_task_id, mergeable, pending_merges,
        pending_releases, pending_reviews, releasable, release_claim, release_task_id, report,
        review_report, review_task_id, set_status, set_status_by_operator, unreviewed, update,
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

    fn even_later() -> time::OffsetDateTime {
        datetime!(2026-03-04 05:06:09 UTC)
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
                archived: false,
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
            release_level: ReleaseLevel::Patch,
            depends_on: None,
        }
    }

    #[test]
    fn transition_table_matches_the_status_vocabulary() {
        use TaskStatus::{
            Approved, Blocked, Cancelled, Done, Draft, Dropped, Merged, Ready, Released, Wip,
        };

        for (from, to) in [
            (Draft, Ready),
            (Ready, Wip),
            (Wip, Done),
            (Wip, Ready),
            (Done, Approved),
            (Approved, Merged),
            (Merged, Released),
            (Blocked, Ready),
            (Draft, Blocked),
            (Merged, Cancelled),
            (Approved, Cancelled),
            (Wip, Dropped),
        ] {
            assert!(can_transition(from, to), "{from:?} -> {to:?} must be legal");
        }

        for (from, to) in [
            (Draft, Wip),
            (Draft, Done),
            (Ready, Done),
            (Done, Merged),
            (Done, Released),
            (Approved, Released),
            (Released, Ready),
            (Released, Blocked),
            (Dropped, Ready),
            (Cancelled, Ready),
            (Ready, Ready),
            (Blocked, Blocked),
            // A human may not reopen finished work by hand: only a review that
            // requests changes sends a `done` task back, through its own
            // operation, and only `approved` moves on to `merged`.
            (Done, Ready),
            (Approved, Ready),
            (Approved, Wip),
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
            claim(&db, "  ", &[], now(), 60),
            Err(Error::Invalid(_))
        ));
        assert!(claim(&db, "worker", &[], now(), 60).unwrap().is_none());

        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();
        let leased = claim(&db, "worker", &[], now(), 60).unwrap().unwrap();
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
        let leased = claim(&db, "worker", &[], now(), 60).unwrap().unwrap();
        let claim_id = leased.claim_id.clone().unwrap();

        let done = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            now(),
        )
        .unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert_eq!(done.commit_sha.as_deref(), Some("abc1234"));

        let again = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            now(),
        )
        .unwrap();
        assert_eq!(again, done);

        assert!(matches!(
            report(
                &db,
                &claim_id,
                "def5678",
                "cargo test",
                &[],
                ReportOutcome::Done,
                None,
                now()
            ),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            report(
                &db,
                "not-a-claim",
                "abc1234",
                "cargo test",
                &[],
                ReportOutcome::Done,
                None,
                now()
            ),
            Err(Error::ClaimMismatch)
        ));
        assert!(matches!(
            report(
                &db,
                &claim_id,
                " ",
                "cargo test",
                &[],
                ReportOutcome::Done,
                None,
                now()
            ),
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
                archived: false,
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

    /// The accident this prevents: a directory was deleted, so the product is
    /// archived, and someone files new work against it anyway. The catalogue still
    /// carries the row — every task that named it must keep resolving — so
    /// "missing" is the wrong answer. The promotion is refused under its own code,
    /// because the remedy is different: restore the clone, do not add a product.
    #[test]
    fn ready_is_refused_while_the_product_is_archived() {
        let db = db_with_product();
        let history = NewTask {
            product_id: Some("a/b".into()),
            ..new_task("t-old", TaskKind::Normal, 0)
        };
        create(&db, &history, now()).unwrap();
        force_status(&db, "t-old", "merged");
        create(&db, &new_task("t-new", TaskKind::Normal, 0), now()).unwrap();

        // The walk stopped finding the working copy of `a/b`, and found another
        // product instead. An empty walk would have archived nothing.
        let other = Product {
            id: "z/z".into(),
            repository: "https://example.test/z/z.git".into(),
            description: String::new(),
            releases: true,
            archived: false,
        };
        let report = product::reconcile(&db, std::slice::from_ref(&other), now()).unwrap();
        assert_eq!(report.archived.len(), 1, "a/b left the tree");
        assert_eq!(report.archived[0].tasks, 2, "both tasks still name it");

        let listed = product::list(&db).unwrap();
        assert_eq!(listed[0].id, "a/b", "the row stays in the catalogue");
        assert!(listed[0].archived);

        // History is untouched: the merged task still resolves its product.
        let old = get(&db, "t-old").unwrap();
        assert_eq!(old.product_id.as_deref(), Some("a/b"));
        assert_eq!(old.status, TaskStatus::Merged);

        let err = set_status(&db, "t-new", TaskStatus::Ready, later()).unwrap_err();
        let Error::Precondition { code, message } = &err else {
            panic!("unexpected error: {err:?}");
        };
        assert_ne!(
            *code, "product_not_catalogued",
            "an archived product is catalogued; the reason is that its clone is gone"
        );
        assert_eq!(*code, "product_archived");
        assert!(message.contains("a/b"), "{message}");
        assert_eq!(
            get(&db, "t-new").unwrap().status,
            TaskStatus::Draft,
            "a refused promotion leaves the row where it was"
        );

        // The remedy is the directory coming back, which the walk undoes.
        let restored = Product {
            id: "a/b".into(),
            repository: "https://example.test/a/b.git".into(),
            description: String::new(),
            releases: true,
            archived: false,
        };
        let report = product::reconcile(&db, &[restored, other], later()).unwrap();
        assert_eq!(report.unarchived, ["a/b"]);
        assert_eq!(
            set_status(&db, "t-new", TaskStatus::Ready, later())
                .unwrap()
                .status,
            TaskStatus::Ready,
            "restoring the clone is the whole remedy"
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
    fn available_transitions_never_offer_approved_merged_or_released() {
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
            can_transition(TaskStatus::Done, TaskStatus::Approved),
            "the control plane still uses the transition table"
        );
        assert!(
            !done.contains(&TaskStatus::Approved),
            "approved is granted by a review that approves, never pressed: {done:?}"
        );
        assert!(
            !done.contains(&TaskStatus::Merged),
            "merged is granted by a green merge report, never pressed: {done:?}"
        );
        assert!(done.contains(&TaskStatus::Blocked));

        set_status(&db, "t-ship", TaskStatus::Approved, now()).unwrap();
        let approved = available_transitions(&get(&db, "t-ship").unwrap());
        assert!(
            !approved.contains(&TaskStatus::Merged),
            "merged is still not pressed from approved: {approved:?}"
        );
        assert!(
            !approved.contains(&TaskStatus::Ready),
            "approved work is not reopened by hand: {approved:?}"
        );
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

        let first = claim(&db, "worker", &[], now(), 60).unwrap().unwrap();
        assert_eq!(first.id, "t-1");
        assert_eq!(first.branch.as_deref(), Some("task/t-1"));

        let second = claim(&db, "worker", &[], now(), 60).unwrap().unwrap();
        assert_eq!(second.id, "t-2");
        assert_eq!(second.branch.as_deref(), Some("feature/manual"));
    }

    #[test]
    fn an_expired_lease_is_reclaimed_with_a_fresh_claim_id() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();

        let first = claim(&db, "worker-a", &[], now(), 60).unwrap().unwrap();
        let abandoned = first.claim_id.clone().unwrap();
        assert_eq!(
            first.claim_expires_at.as_deref(),
            Some("2026-03-04T05:07:07Z")
        );

        // A lease that has not expired yet belongs to the worker holding it.
        let alive = now() + time::Duration::seconds(59);
        assert!(claim(&db, "worker-b", &[], alive, 60).unwrap().is_none());

        let expired = now() + time::Duration::seconds(61);
        let retaken = claim(&db, "worker-b", &[], expired, 60).unwrap().unwrap();
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
                report(
                    &db,
                    &abandoned,
                    "abc1234",
                    "cargo test",
                    &[],
                    ReportOutcome::Done,
                    None,
                    expired
                ),
                Err(Error::ClaimMismatch)
            ),
            "the abandoned lease must no longer report"
        );
        assert_eq!(
            report(
                &db,
                &fresh,
                "abc1234",
                "cargo test",
                &[],
                ReportOutcome::Done,
                None,
                expired
            )
            .unwrap()
            .status,
            TaskStatus::Done
        );

        // A task that left `wip` is never handed out again by expiry. The
        // filter is what keeps this about `t-1`: finishing it issued the review
        // that reads it, and that review is waiting for a reviewer.
        let far_future = now() + time::Duration::seconds(100_000);
        assert!(
            claim(&db, "worker-c", &[TaskKind::Normal], far_future, 60)
                .unwrap()
                .is_none()
        );
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
            claim(&db, "worker", &[], far_future, 60).unwrap().is_none(),
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
            TaskStatus::Approved,
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

    /// Take `id` from draft to done the way a worker does. The filter is what
    /// keeps a worker loop off the reviews earlier work has already queued.
    fn work_to_done(db: &Db, id: &str) {
        set_status(db, id, TaskStatus::Ready, now()).unwrap();
        let leased = claim(db, "worker", &[TaskKind::Normal], now(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, id);
        let claim_id = leased.claim_id.expect("claim_id");
        report(
            db,
            &claim_id,
            "abc1234",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            now(),
        )
        .unwrap();
    }

    /// Claim the review the report of `target_id` issued, the way a reviewer
    /// does. The review is already waiting: finishing the work is what filed it.
    fn claim_review(db: &Db, target_id: &str) -> (String, String) {
        let leased = claim(db, "reviewer", &[TaskKind::Review], later(), 60)
            .unwrap()
            .expect("the report must have issued a review to claim");
        assert_eq!(
            leased.review_target_task_id.as_deref(),
            Some(target_id),
            "the reviewer must get the review of {target_id}"
        );
        (leased.id.clone(), leased.claim_id.expect("claim_id"))
    }

    /// The merge the approval of `target_id` issued.
    fn issued_merge(db: &Db, target_id: &str) -> Task {
        get(db, &merge_task_id(target_id)).expect("the approval must have issued a merge")
    }

    /// Take `id` all the way to `approved`: work, then a review that approves
    /// the commit the work reported. Nothing else grants that status.
    fn work_to_approved(db: &Db, id: &str) {
        work_to_done(db, id);
        let (_, claim_id) = claim_review(db, id);
        review_report(
            db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "read the diff, ran the tests",
            later(),
        )
        .unwrap();
        assert_eq!(get(db, id).unwrap().status, TaskStatus::Approved);
    }

    #[test]
    fn only_approved_normal_work_without_a_live_merge_is_mergeable() {
        let db = db_with_product();
        for id in ["t-done", "t-ready", "t-draft"] {
            create(&db, &new_task(id, TaskKind::Normal, 0), now()).unwrap();
        }
        set_status(&db, "t-ready", TaskStatus::Ready, now()).unwrap();
        work_to_approved(&db, "t-done");

        // The approval issued the merge, so the candidate list is empty and the
        // work is already in flight rather than waiting for a press.
        assert!(
            mergeable(&db).unwrap().is_empty(),
            "approved work with a live merge is not a candidate"
        );

        let merge = issued_merge(&db, "t-done");
        assert_eq!(merge.id, merge_task_id("t-done"));
        assert_eq!(merge.kind, TaskKind::InstantMerge);
        assert_eq!(merge.status, TaskStatus::Ready);
        assert_eq!(merge.merge_target_task_id.as_deref(), Some("t-done"));
        assert_eq!(merge.product_id.as_deref(), Some("a/b"));
        assert_eq!(merge.branch.as_deref(), Some("task/t-done"));
        assert_eq!(merge.commit_sha.as_deref(), Some("abc1234"));
        assert!(merge.title.contains("t-done"));

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
        work_to_approved(&db, "t-1");
        let merge = issued_merge(&db, "t-1");
        let leased = claim(&db, "worker", &[], later(), 60).unwrap().unwrap();
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
                    report(
                        &db,
                        &claim_id,
                        "abc1234",
                        "cargo test",
                        &checks,
                        ReportOutcome::Done,
                        None,
                        later()
                    ),
                    Err(Error::Invalid(_))
                ),
                "{checks:?} must not land"
            );
            assert_eq!(get(&db, &merge.id).unwrap().status, TaskStatus::Wip);
            assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Approved);
        }

        let landed = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(landed.status, TaskStatus::Done);
        assert_eq!(landed.checks, green(), "the evidence is kept on the task");
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);

        // Idempotent: the same commit reported twice is still accepted.
        let again = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(again, landed);
    }

    /// The gate is not a one-time door. Once a merge is `done`, a repeat report
    /// still has to carry the same evidence, or a worker could report "no
    /// checks" against a landed merge and read the 200 as a pass.
    #[test]
    fn a_landed_merge_still_refuses_a_report_without_green_checks() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_approved(&db, "t-1");
        let merge = issued_merge(&db, "t-1");
        let claim_id = claim(&db, "worker", &[], later(), 60)
            .unwrap()
            .unwrap()
            .claim_id
            .expect("claim_id");
        let landed = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
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
                    report(
                        &db,
                        &claim_id,
                        "abc1234",
                        "cargo test",
                        &checks,
                        ReportOutcome::Done,
                        None,
                        later()
                    ),
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

        let again = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(again, landed, "a green repeat is still idempotent");
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);
    }

    /// The index frees a target once its merge is cancelled, so the id rule has
    /// to leave room for the retry the index allows.
    #[test]
    fn a_cancelled_merge_is_reissued_under_a_new_id() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_approved(&db, "t-1");

        let first = issued_merge(&db, "t-1");
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
        work_to_approved(&db, "t-1");
        let merge = issued_merge(&db, "t-1");
        let leased = claim(&db, "worker", &[], later(), 60).unwrap().unwrap();
        let claim_id = leased.claim_id.expect("claim_id");

        set_status(&db, "t-1", TaskStatus::Merged, later()).unwrap();
        assert!(matches!(
            report(
                &db,
                &claim_id,
                "abc1234",
                "cargo test",
                &green(),
                ReportOutcome::Done,
                None,
                later()
            ),
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
        let claim_id = claim(&db, "worker", &[], now(), 60)
            .unwrap()
            .unwrap()
            .claim_id
            .unwrap();

        let done = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            now(),
        )
        .unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert_eq!(done.checks, green());
        assert_eq!(get(&db, "t-1").unwrap().checks, green());
    }

    fn claim_release(db: &Db) -> Task {
        claim(db, "shipper", &[TaskKind::InstantRelease], later(), 60)
            .unwrap()
            .expect("a release must be waiting to be claimed")
    }

    fn ship(db: &Db, release: &Task, tag: Option<&str>) -> Result<Task, Error> {
        report(
            db,
            release.claim_id.as_deref().expect("claim_id"),
            "abc9999",
            "bump-tag",
            &green(),
            ReportOutcome::Done,
            tag,
            later(),
        )
    }

    /// The landing is where the release is decided: the merge that lands the
    /// work issues the release that ships it, in the same transaction, and
    /// points the work at it. Nothing is stranded and nobody pressed anything.
    #[test]
    fn landing_issues_the_release_and_points_the_work_at_it() {
        let db = db_with_product();
        let merge = merge_waiting_for(&db, "t-1", "a/b");
        assert_eq!(
            get(&db, &merge.id).unwrap().release_level,
            ReleaseLevel::Patch,
            "a subtask inherits the level of its target"
        );
        merge_into(&db, &merge.id, TaskStatus::Done);

        let release = get(&db, &release_task_id("t-1")).unwrap();
        assert_eq!(release.kind, TaskKind::InstantRelease);
        assert_eq!(release.status, TaskStatus::Ready);
        assert_eq!(release.product_id.as_deref(), Some("a/b"));
        assert_eq!(release.release_level, ReleaseLevel::Patch);
        assert_eq!(release.title, "release a/b: patch");
        assert!(release.commit_sha.is_none());
        assert_eq!(
            get(&db, "t-1").unwrap().release_task_id.as_deref(),
            Some(release.id.as_str())
        );
        assert_eq!(
            pending_releases(&db)
                .unwrap()
                .iter()
                .map(|task| task.id.clone())
                .collect::<Vec<_>>(),
            std::slice::from_ref(&release.id)
        );
        assert!(
            releasable(&db).unwrap().is_empty(),
            "work with a release carrying it is not stranded"
        );
        assert!(
            matches!(issue_release(&db, "a/b", later()), Err(Error::Conflict(_))),
            "a release in flight is not issued twice"
        );
    }

    /// While a release is open, the next landing issues nothing; the report
    /// that ends the release ships everything it carried — and its subtasks —
    /// under the tag it cut, then gathers what landed in the meantime.
    #[test]
    fn a_release_report_ships_its_work_and_gathers_what_landed_meanwhile() {
        let db = db_with_product();
        let first = merge_waiting_for(&db, "t-1", "a/b");
        merge_into(&db, &first.id, TaskStatus::Done);
        let release = claim_release(&db);
        assert_eq!(release.id, release_task_id("t-1"));

        create(
            &db,
            &NewTask {
                release_level: ReleaseLevel::Minor,
                ..new_task("t-2", TaskKind::Normal, 0)
            },
            now(),
        )
        .unwrap();
        work_to_approved(&db, "t-2");
        let second = issued_merge(&db, "t-2");
        merge_into(&db, &second.id, TaskStatus::Done);
        assert_eq!(get(&db, "t-2").unwrap().status, TaskStatus::Merged);
        assert!(
            get(&db, "t-2").unwrap().release_task_id.is_none(),
            "an open release holds the next landing back"
        );
        assert_eq!(pending_releases(&db).unwrap().len(), 1);
        assert!(
            releasable(&db).unwrap().is_empty(),
            "work waiting on an open release is not stranded"
        );

        let shipped = ship(&db, &release, Some("v0.1.1")).unwrap();
        assert_eq!(shipped.status, TaskStatus::Released);
        assert_eq!(shipped.release_tag.as_deref(), Some("v0.1.1"));
        assert_eq!(shipped.commit_sha.as_deref(), Some("abc9999"));
        for id in ["t-1", &review_task_id("t-1"), &first.id] {
            let task = get(&db, id).unwrap();
            assert_eq!(task.status, TaskStatus::Released, "{id}");
            assert_eq!(task.release_tag.as_deref(), Some("v0.1.1"), "{id}");
        }

        // The repeat of the same report is accepted and moves nothing.
        assert_eq!(
            ship(&db, &release, Some("v0.1.1")).unwrap().status,
            TaskStatus::Released
        );

        let next = get(&db, &release_task_id("t-2")).unwrap();
        assert_eq!(next.status, TaskStatus::Ready);
        assert_eq!(next.release_level, ReleaseLevel::Minor);
        assert_eq!(
            get(&db, "t-2").unwrap().release_task_id.as_deref(),
            Some(next.id.as_str())
        );
        assert_eq!(get(&db, "t-2").unwrap().status, TaskStatus::Merged);
    }

    /// A target that went through review twice (`request_changes`, then approve)
    /// carries two finished reviews. Shipping it moves both to `released` in
    /// one UPDATE — which the one-open-review index used to refuse, because its
    /// predicate treated `released` as open and two released reviews of one
    /// target collided. The whole report rolled back as a 500 and the release
    /// stayed `wip` while the tag was already on origin.
    #[test]
    fn a_release_ships_a_target_that_was_reviewed_twice() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (first_review, claim_id) = claim_review(&db, "t-1");
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::RequestChanges,
            "please add the missing test",
            later(),
        )
        .unwrap();
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Ready);
        // Second round: the worker reports a new commit, a second review approves.
        let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .expect("the work handed back is claimable again");
        assert_eq!(leased.id, "t-1");
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "def5678",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        let (second_review, claim_id) = claim_review(&db, "t-1");
        assert_eq!(second_review, format!("{first_review}~2"));
        review_report(
            &db,
            &claim_id,
            "def5678",
            ReviewVerdict::Approve,
            "now it is right",
            later(),
        )
        .unwrap();
        let merge = issued_merge(&db, "t-1");
        merge_into(&db, &merge.id, TaskStatus::Done);
        let release = claim_release(&db);

        let shipped = ship(&db, &release, Some("v0.1.1")).expect("two finished reviews ship");
        assert_eq!(shipped.status, TaskStatus::Released);
        for id in ["t-1", &first_review, &second_review, &merge.id, &release.id] {
            let task = get(&db, id).unwrap();
            assert_eq!(task.status, TaskStatus::Released, "{id}");
            assert_eq!(task.release_tag.as_deref(), Some("v0.1.1"), "{id}");
        }
        assert!(
            pending_reviews(&db).unwrap().is_empty(),
            "released reviews are not pending"
        );
        assert!(
            pending_merges(&db).unwrap().is_empty(),
            "released merges are not pending"
        );
        assert!(pending_releases(&db).unwrap().is_empty());
    }

    /// A release is finished by the tag it cut and by nothing else: no tag, or
    /// one that is not a version, is refused and the row does not move.
    #[test]
    fn a_release_report_without_a_version_tag_is_refused() {
        let db = db_with_product();
        let merge = merge_waiting_for(&db, "t-1", "a/b");
        merge_into(&db, &merge.id, TaskStatus::Done);
        let release = claim_release(&db);

        for tag in [None, Some(""), Some("1.2.3"), Some("v1.2"), Some("v1.2.x")] {
            assert!(
                matches!(ship(&db, &release, tag), Err(Error::Invalid(_))),
                "{tag:?} is not a release tag"
            );
        }
        assert_eq!(get(&db, &release.id).unwrap().status, TaskStatus::Wip);
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);
    }

    /// A product that does not release ends at the landing: the work and its
    /// finished subtasks are released with no tag, and no release is issued.
    #[test]
    fn a_product_that_does_not_release_ends_at_the_landing() {
        let db = db_with_product();
        product::upsert(
            &db,
            &Product {
                id: "c/d".into(),
                repository: "https://example.test/c/d.git".into(),
                description: String::new(),
                releases: false,
                archived: false,
            },
            now(),
        )
        .unwrap();
        let merge = merge_waiting_for(&db, "t-keep", "c/d");
        assert_eq!(
            claim_merge(&db, "luna", later()).as_deref(),
            Some(merge.id.as_str())
        );
        land_merge(&db, &merge.id, "abc1234");

        for id in ["t-keep", &review_task_id("t-keep"), &merge.id] {
            let task = get(&db, id).unwrap();
            assert_eq!(task.status, TaskStatus::Released, "{id}");
            assert!(task.release_tag.is_none(), "{id}");
        }
        assert!(pending_releases(&db).unwrap().is_empty());
        assert!(matches!(
            get(&db, &release_task_id("t-keep")),
            Err(Error::NotFound)
        ));
        assert!(matches!(
            issue_release(&db, "c/d", later()),
            Err(Error::Conflict(_))
        ));
    }

    /// A release that was called off leaves its work merged with no carrier,
    /// and the next landing gathers all of it under one release whose level
    /// is the largest of the work it ships and whose id names the newest.
    #[test]
    fn the_release_level_is_the_largest_of_the_work_it_ships() {
        let db = db_with_product();
        let first = merge_waiting_for(&db, "t-1", "a/b");
        merge_into(&db, &first.id, TaskStatus::Done);
        set_status_by_operator(&db, &release_task_id("t-1"), TaskStatus::Cancelled, later())
            .unwrap();
        assert_eq!(
            releasable(&db).unwrap(),
            vec![Releasable {
                product_id: "a/b".into(),
                task_count: 1,
            }],
            "work whose release was called off is stranded"
        );

        create(
            &db,
            &NewTask {
                release_level: ReleaseLevel::Major,
                ..new_task("t-2", TaskKind::Normal, 3)
            },
            later(),
        )
        .unwrap();
        work_to_approved(&db, "t-2");
        merge_into(&db, &issued_merge(&db, "t-2").id, TaskStatus::Done);

        let release = get(&db, &release_task_id("t-2")).unwrap();
        assert_eq!(release.release_level, ReleaseLevel::Major);
        assert_eq!(
            release.priority, 3,
            "a release is not handed out behind its work"
        );
        for id in ["t-1", "t-2"] {
            assert_eq!(
                get(&db, id).unwrap().release_task_id.as_deref(),
                Some(release.id.as_str()),
                "{id} is carried by the new release"
            );
        }
        assert!(releasable(&db).unwrap().is_empty());
    }

    /// A blocked release is written down like a blocked merge: the work stays
    /// merged, the row keeps the reason, and the way out is calling the attempt
    /// off and issuing a new one by hand.
    #[test]
    fn a_blocked_release_keeps_its_work_merged_and_is_reissued_by_hand() {
        let db = db_with_product();
        let merge = merge_waiting_for(&db, "t-1", "a/b");
        merge_into(&db, &merge.id, TaskStatus::Done);
        let release = claim_release(&db);

        let blocked = report(
            &db,
            release.claim_id.as_deref().unwrap(),
            "abc1234",
            "bump-tag: the tag already exists",
            &[Check {
                name: "bump-tag".into(),
                exit_code: 1,
            }],
            ReportOutcome::Blocked,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(blocked.status, TaskStatus::Blocked);
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);
        assert_eq!(
            available_transitions(&blocked),
            [TaskStatus::Cancelled, TaskStatus::Dropped],
            "a blocked release is called off, never restarted"
        );
        assert!(matches!(
            issue_release(&db, "a/b", later()),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            issue_release(&db, "x/y", later()),
            Err(Error::NotFound)
        ));

        set_status_by_operator(&db, &release.id, TaskStatus::Dropped, later()).unwrap();
        let again = issue_release(&db, "a/b", later()).unwrap();
        assert_eq!(again.id, format!("{}~2", release_task_id("t-1")));
        assert_eq!(again.status, TaskStatus::Ready);
        assert_eq!(
            get(&db, "t-1").unwrap().release_task_id.as_deref(),
            Some(again.id.as_str()),
            "the work is pointed at the new attempt"
        );
    }

    /// An archived product releases nothing, whatever its stored `releases`
    /// says. The flag is derived from the clone's `.github/workflows`, and an
    /// archived product has no clone in the tree to derive it from any more —
    /// the row keeps the last answer, which is exactly the answer that can no
    /// longer be checked. Releasing is also the one operation that most needs
    /// the working copy: the CI that builds the artefacts runs from it.
    ///
    /// So the landing leaves the work merged and issues nothing, and a clone put
    /// back makes it releasable again on the next walk with nobody re-entering
    /// anything.
    #[test]
    fn an_archived_product_releases_nothing_until_its_clone_is_back() {
        let db = db_with_product();
        let elsewhere = Product {
            id: "c/d".into(),
            repository: "https://example.test/c/d.git".into(),
            description: String::new(),
            releases: true,
            archived: false,
        };
        let on_disk = Product {
            id: "a/b".into(),
            repository: "https://example.test/a/b.git".into(),
            description: String::new(),
            releases: true,
            archived: false,
        };

        let merge = merge_waiting_for(&db, "t-1", "a/b");

        // The walk no longer finds `a/b`, so its row is archived.
        product::reconcile(&db, std::slice::from_ref(&elsewhere), later()).unwrap();
        let archived = product::get(&db, "a/b").unwrap();
        assert!(archived.archived);
        assert!(
            archived.releases,
            "the stored flag is left exactly as the last walk wrote it"
        );

        merge_into(&db, &merge.id, TaskStatus::Done);
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);
        assert!(pending_releases(&db).unwrap().is_empty());
        assert!(
            releasable(&db).unwrap().is_empty(),
            "an archived product is not offered as release-ready"
        );
        assert!(matches!(
            issue_release(&db, "a/b", later()),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            set_status(&db, "t-1", TaskStatus::Released, later()),
            Err(Error::Invalid(_))
        ));

        // The clone comes back, and so does the release.
        product::reconcile(&db, &[elsewhere, on_disk], later()).unwrap();
        assert_eq!(
            releasable(&db).unwrap(),
            vec![Releasable {
                product_id: "a/b".into(),
                task_count: 1,
            }]
        );
        let release = issue_release(&db, "a/b", later()).unwrap();
        assert_eq!(release.id, release_task_id("t-1"));
        assert!(releasable(&db).unwrap().is_empty());
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

        // The internal path is untouched: a merge still comes from a target,
        // issued by the approval that earned it.
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_approved(&db, "t-1");
        let merge = issued_merge(&db, "t-1");
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

        let leased = claim(&db, "worker", &[], now(), 60).unwrap().unwrap();
        assert_eq!(leased.id, "t-orphan");
        let claim_id = leased.claim_id.expect("claim_id");

        let refused = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
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
    /// `approved`, `merged`, and `released`, while the control plane goes on
    /// using `set_status`. `approved` is the reviewer's to grant, and a human
    /// who could press it would be approving their own work.
    #[test]
    fn an_operator_may_not_press_approved_merged_or_released() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");

        for to in [
            TaskStatus::Approved,
            TaskStatus::Merged,
            TaskStatus::Released,
        ] {
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
        set_status(&db, "t-1", TaskStatus::Approved, later()).unwrap();
        let merged = set_status(&db, "t-1", TaskStatus::Merged, later()).unwrap();
        assert_eq!(merged.status, TaskStatus::Merged);
    }

    /// The completion contract of a review has to be unavoidable, and `done` is
    /// the press that would walk around it: it would finish the review with no
    /// verdict and no findings, and — because the single-open-review index stops
    /// at `done` — free the target for the next review as though this one had
    /// answered. Calling a review off is a different act and stays pressable.
    #[test]
    fn an_operator_may_not_finish_a_review_by_pressing_done() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let review = get(&db, &review_task_id("t-1")).expect("the report issues a review");

        // Refused while the review is still queued, and refused again while a
        // reviewer holds it — which is the only place the transition table
        // would otherwise have let `done` through.
        let queued = set_status_by_operator(&db, &review.id, TaskStatus::Done, later())
            .expect_err("a review is not finished by a press");
        assert!(
            matches!(&queued, Error::Invalid(message) if message.contains("verdict")),
            "the refusal must name the verdict: {queued:?}"
        );

        let leased = claim(&db, "sol", &[TaskKind::Review], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, review.id, "the reviewer must hold the review");
        let held = set_status_by_operator(&db, &review.id, TaskStatus::Done, later())
            .expect_err("a review is not finished by a press");
        assert!(
            matches!(&held, Error::Invalid(message) if message.contains("verdict")),
            "the refusal must name the verdict: {held:?}"
        );

        let review = get(&db, &review.id).unwrap();
        assert_eq!(review.status, TaskStatus::Wip, "a refusal moves no row");
        assert!(
            review.review_verdict.is_none(),
            "and leaves the review with nothing to say"
        );
        assert_eq!(
            get(&db, "t-1").unwrap().status,
            TaskStatus::Done,
            "and answers nothing for the parent"
        );
        assert!(
            latest_review(&db, "t-1").unwrap().is_none(),
            "a pressed review is not a finished review"
        );
        assert!(
            matches!(issue_review(&db, "t-1", later()), Err(Error::Conflict(_))),
            "the refused press must not have freed the one-open-review index"
        );

        let offered = available_transitions(&review);
        assert!(
            !offered.contains(&TaskStatus::Done),
            "a status nobody may press is never offered either: {offered:?}"
        );
        assert!(
            offered.contains(&TaskStatus::Ready),
            "handing the review back to the queue is still a press: {offered:?}"
        );

        // Ordinary work keeps its `done`: the refusal is about reviews only.
        create(&db, &new_task("t-2", TaskKind::Normal, 0), now()).unwrap();
        set_status_by_operator(&db, "t-2", TaskStatus::Ready, later()).unwrap();
        set_status_by_operator(&db, "t-2", TaskStatus::Wip, later()).unwrap();
        let finished = set_status_by_operator(&db, "t-2", TaskStatus::Done, later()).unwrap();
        assert_eq!(finished.status, TaskStatus::Done);

        // Calling a review off stays open. `blocked` keeps holding the target,
        // because the attempt is still in flight; `cancelled` and `dropped`
        // release it on purpose, which is what abandoning an attempt means.
        set_status_by_operator(&db, &review.id, TaskStatus::Blocked, later()).unwrap();
        assert!(
            matches!(issue_review(&db, "t-1", later()), Err(Error::Conflict(_))),
            "a blocked review is still the open one"
        );
        set_status_by_operator(&db, &review.id, TaskStatus::Cancelled, later()).unwrap();
        let second =
            issue_review(&db, "t-1", later()).expect("a cancelled attempt frees the target");
        assert_eq!(second.id, "review:t-1~2");
        set_status_by_operator(&db, &second.id, TaskStatus::Dropped, later()).unwrap();
        let third = issue_review(&db, "t-1", later()).expect("so does a dropped one");
        assert_eq!(third.id, "review:t-1~3");
        assert!(
            latest_review(&db, "t-1").unwrap().is_none(),
            "an abandoned attempt still answered nothing"
        );
    }

    /// A review that answered is over, and an operator press must not raise it.
    ///
    /// Two things break if one can. `blocked` puts the finished attempt back
    /// inside the single-open-review index — whose predicate stops at `done`,
    /// `cancelled` and `dropped` — so an attempt that already did its job would
    /// stand in the way of the next review of the same target. And from
    /// `blocked` the row walks on to `ready` and, on a claim, back to `wip`,
    /// where `review_report` would accept it a second time and write a new
    /// verdict over the one the target lives by.
    #[test]
    fn an_answered_review_is_terminal_for_an_operator() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (review_id, claim_id) = claim_review(&db, "t-1");
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::RequestChanges,
            "the guard is missing on the empty case",
            later(),
        )
        .unwrap();

        let answered = get(&db, &review_id).unwrap();
        assert_eq!(answered.status, TaskStatus::Done);
        assert_eq!(answered.review_verdict, Some(ReviewVerdict::RequestChanges));
        let offered = available_transitions(&answered);
        assert!(
            offered.is_empty(),
            "an answered review offers nothing to press: {offered:?}"
        );

        // Every status in the vocabulary, refused, and nothing written.
        for to in ALL_STATUSES {
            let refused = set_status_by_operator(&db, &review_id, to, later())
                .expect_err("an answered review moves no further");
            assert!(
                matches!(&refused, Error::Invalid(_)),
                "unexpected error pressing {}: {refused:?}",
                to.as_str()
            );
            assert_eq!(
                get(&db, &review_id).unwrap(),
                answered,
                "the refusal of {} must write nothing",
                to.as_str()
            );
        }
        // The three the transition table would have let through are refused for
        // being an answer, not for being granted elsewhere.
        for to in [
            TaskStatus::Blocked,
            TaskStatus::Cancelled,
            TaskStatus::Dropped,
        ] {
            let refused = set_status_by_operator(&db, &review_id, to, later()).unwrap_err();
            assert!(
                matches!(&refused, Error::Invalid(message) if message.contains("already answered")),
                "the refusal of {} must name the answer: {refused:?}",
                to.as_str()
            );
        }

        let outcome = latest_review(&db, "t-1").unwrap().expect("a verdict");
        assert_eq!(outcome.verdict, ReviewVerdict::RequestChanges);
        assert_eq!(
            get(&db, "t-1").unwrap().status,
            TaskStatus::Ready,
            "the verdict the parent lives by stands"
        );
        assert!(
            claim(&db, "sol", &[TaskKind::Review], later(), 60)
                .unwrap()
                .is_none(),
            "and the answered review cannot be leased again"
        );
    }

    /// The other half of the same rule: an approval is as final as a request for
    /// changes, and a frozen attempt never stands in the way of the next one.
    #[test]
    fn an_approving_review_is_final_too_and_blocks_no_later_attempt() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (first_id, claim_id) = claim_review(&db, "t-1");
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::RequestChanges,
            "the guard is missing on the empty case",
            later(),
        )
        .unwrap();
        let frozen = get(&db, &first_id).unwrap();
        assert!(available_transitions(&frozen).is_empty());

        // The rework is reviewed, and that review is issued while the first
        // attempt sits frozen.
        let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, "t-1");
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "def5678",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        let (second_id, second_claim) = claim_review(&db, "t-1");
        assert_eq!(second_id, "review:t-1~2");
        review_report(
            &db,
            &second_claim,
            "def5678",
            ReviewVerdict::Approve,
            "the guard is there now",
            later(),
        )
        .unwrap();
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Approved);

        // An approving review is just as final as one that asked for changes.
        let second = get(&db, &second_id).unwrap();
        assert!(
            available_transitions(&second).is_empty(),
            "an approval is a record too: {:?}",
            available_transitions(&second)
        );
        assert!(matches!(
            set_status_by_operator(&db, &second_id, TaskStatus::Blocked, later()),
            Err(Error::Invalid(_))
        ));
        assert_eq!(get(&db, &second_id).unwrap(), second);
        assert_eq!(
            latest_review(&db, "t-1")
                .unwrap()
                .expect("a verdict")
                .verdict,
            ReviewVerdict::Approve
        );
        assert_eq!(
            get(&db, &first_id).unwrap(),
            frozen,
            "and the earlier attempt is still exactly where it answered"
        );
    }

    /// The merge side of the hazard the review snapshot exists for: `approved`
    /// does not say *which* commit was approved. A merge issued for commit A can
    /// arrive after the work was reopened, redone as B, reviewed and approved
    /// again — and landing it would mark the task merged for a commit nobody
    /// approved.
    #[test]
    fn a_merge_of_a_commit_the_parent_has_left_behind_lands_nothing() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_approved(&db, "t-1");
        let merge = issued_merge(&db, "t-1");
        assert_eq!(
            merge.commit_sha.as_deref(),
            Some("abc1234"),
            "the merge snapshots the commit it was issued for"
        );

        // The work is taken back, redone on another commit, reviewed again and
        // approved again. All of it legitimate, none of it known to the merge
        // that is still in flight.
        set_status_by_operator(&db, "t-1", TaskStatus::Blocked, later()).unwrap();
        set_status_by_operator(&db, "t-1", TaskStatus::Ready, later()).unwrap();
        let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, "t-1");
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "def5678",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        let (_, review_claim) = claim_review(&db, "t-1");
        review_report(
            &db,
            &review_claim,
            "def5678",
            ReviewVerdict::Approve,
            "read the new diff",
            later(),
        )
        .unwrap();
        let parent = get(&db, "t-1").unwrap();
        assert_eq!(parent.status, TaskStatus::Approved);
        assert_eq!(parent.commit_sha.as_deref(), Some("def5678"));

        // The stale merge is claimed and reported green. Its checks passed and
        // its target is `approved`; the commit is the only thing that says no.
        let leased = claim(&db, "merger", &[TaskKind::InstantMerge], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, merge.id);
        let refused = report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "merge999",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
        .expect_err("a merge of a commit the parent left behind must be refused");
        assert!(
            matches!(&refused, Error::Precondition { code, .. } if *code == "merge_subject_changed"),
            "unexpected error: {refused:?}"
        );

        let stale = get(&db, &merge.id).unwrap();
        assert_eq!(stale.status, TaskStatus::Wip, "the refusal writes nothing");
        assert_eq!(
            stale.commit_sha.as_deref(),
            Some("abc1234"),
            "and does not take the commit the report carried"
        );
        assert!(stale.verification.is_none());
        assert!(stale.checks.is_empty());
        assert_eq!(get(&db, "t-1").unwrap(), parent, "the target is untouched");

        // A merge issued for the commit the review approved still lands.
        set_status_by_operator(&db, &merge.id, TaskStatus::Cancelled, later()).unwrap();
        let fresh = issue_merge(&db, "t-1", later()).unwrap();
        assert_eq!(fresh.id, "merge:t-1~2");
        assert_eq!(fresh.commit_sha.as_deref(), Some("def5678"));
        let leased = claim(&db, "merger", &[TaskKind::InstantMerge], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, fresh.id);
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "merge999",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);
    }

    /// The tenth attempt is the latest one, not the second. Retry ids are
    /// derived — `review:t-1~9`, `review:t-1~10` — and compare the wrong way
    /// round as text, while the timestamps cannot break the tie because
    /// [`format_z`] writes whole seconds. The attempt number is what decides.
    #[test]
    fn the_latest_review_is_the_highest_attempt_not_the_highest_id_text() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");

        // Nine attempts that hand the work back, each one reworked and finished
        // again, so the tenth is an attempt the control plane really reached.
        for attempt in 1..=9 {
            let (_, claim_id) = claim_review(&db, "t-1");
            review_report(
                &db,
                &claim_id,
                "abc1234",
                ReviewVerdict::RequestChanges,
                &format!("attempt {attempt} wants another pass"),
                later(),
            )
            .unwrap();
            let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
                .unwrap()
                .unwrap();
            report(
                &db,
                &leased.claim_id.expect("claim_id"),
                "abc1234",
                "cargo test",
                &[],
                ReportOutcome::Done,
                None,
                later(),
            )
            .unwrap();
        }

        let (tenth_id, claim_id) = claim_review(&db, "t-1");
        assert_eq!(tenth_id, "review:t-1~10");
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "the tenth pass is good",
            later(),
        )
        .unwrap();

        let ninth = get(&db, "review:t-1~9").unwrap();
        let tenth = get(&db, "review:t-1~10").unwrap();
        assert_eq!(
            ninth.updated_at, tenth.updated_at,
            "the fixture has to be the tie this test is about: same second"
        );
        assert!(
            tenth.id < ninth.id,
            "and the newer attempt has to sort first as text, or the hazard is gone: \
             {} vs {}",
            tenth.id,
            ninth.id
        );

        let outcome = latest_review(&db, "t-1").unwrap().expect("a verdict");
        assert_eq!(outcome.review_task_id, "review:t-1~10");
        assert_eq!(outcome.verdict, ReviewVerdict::Approve);
        assert_eq!(outcome.findings.as_deref(), Some("the tenth pass is good"));
        assert_eq!(
            get(&db, "t-1").unwrap().status,
            TaskStatus::Approved,
            "and the tenth verdict is the one the parent lives by"
        );
    }

    /// A review is issued against finished work and remembers the commit it was
    /// issued for. That snapshot is the whole point: the approval that arrives
    /// later has to be an approval of *this* commit.
    #[test]
    fn a_review_snapshots_the_parents_commit_and_only_one_is_open() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        create(&db, &new_task("t-2", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");

        let review = get(&db, &review_task_id("t-1")).expect("the report issues a review");
        assert_eq!(review.kind, TaskKind::Review);
        assert_eq!(review.status, TaskStatus::Ready);
        assert_eq!(review.review_target_task_id.as_deref(), Some("t-1"));
        assert_eq!(
            review.commit_sha.as_deref(),
            get(&db, "t-1").unwrap().commit_sha.as_deref(),
            "the review carries the commit its parent reported"
        );
        assert_eq!(review.branch.as_deref(), Some("task/t-1"));
        assert_eq!(review.product_id.as_deref(), Some("a/b"));
        assert!(review.review_verdict.is_none());
        assert!(
            review.merge_target_task_id.is_none(),
            "a review is not a merge"
        );

        assert!(
            matches!(issue_review(&db, "t-1", later()), Err(Error::Conflict(_))),
            "one open review per target"
        );
        assert!(
            matches!(issue_review(&db, "t-2", later()), Err(Error::Invalid(_))),
            "unfinished work has nothing to review"
        );
        assert!(matches!(
            issue_review(&db, "t-missing", later()),
            Err(Error::NotFound)
        ));
        assert!(
            matches!(
                issue_review(&db, &review.id, later()),
                Err(Error::Invalid(_))
            ),
            "only normal work is reviewed"
        );
        assert!(
            matches!(
                create(&db, &new_task("t-forged", TaskKind::Review, 0), now()),
                Err(Error::Invalid(_))
            ),
            "a review is issued by the control plane, never filed by hand"
        );
    }

    /// The index that keeps one open review per target must not keep the *next*
    /// one out: a review that finished — approved or not — is over.
    #[test]
    fn a_finished_review_frees_the_target_for_the_next_attempt() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");

        let (first_id, claim_id) = claim_review(&db, "t-1");
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::RequestChanges,
            "the guard is missing",
            later(),
        )
        .unwrap();
        assert_eq!(get(&db, &first_id).unwrap().status, TaskStatus::Done);

        // The rework lands a new commit, and the next review is issued for it.
        let leased = claim(&db, "worker", &[], later(), 60).unwrap().unwrap();
        assert_eq!(leased.id, "t-1");
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "def5678",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();

        let second = get(&db, &format!("{first_id}~2")).expect("the report issues the next one");
        assert!(
            !second.id.contains('/'),
            "a task id is one path segment: {}",
            second.id
        );
        assert_eq!(
            second.commit_sha.as_deref(),
            Some("def5678"),
            "the second review is of the reworked commit"
        );
        assert!(
            matches!(issue_review(&db, "t-1", later()), Err(Error::Conflict(_))),
            "the retry is itself the one open review now"
        );
    }

    /// The approval and the parent's promotion are one write. `approved` is
    /// reachable no other way.
    #[test]
    fn an_approval_promotes_the_parent_in_the_same_transaction() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (review_id, claim_id) = claim_review(&db, "t-1");

        let reported = review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "read the diff, ran the tests",
            later(),
        )
        .unwrap();
        assert_eq!(reported.id, review_id);
        assert_eq!(reported.status, TaskStatus::Done);
        assert_eq!(reported.review_verdict, Some(ReviewVerdict::Approve));
        assert_eq!(
            reported.verification.as_deref(),
            Some("read the diff, ran the tests")
        );
        assert_eq!(
            reported.commit_sha.as_deref(),
            Some("abc1234"),
            "the completion never rewrites the snapshot"
        );
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Approved);

        // Repeating the same verdict for the same commit is accepted and moves
        // nothing; a different verdict on a finished review is not.
        let again = review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "read the diff, ran the tests",
            later(),
        )
        .unwrap();
        assert_eq!(again, reported);
        assert!(matches!(
            review_report(
                &db,
                &claim_id,
                "abc1234",
                ReviewVerdict::RequestChanges,
                "on second thought",
                later()
            ),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            review_report(
                &db,
                "not-a-claim",
                "abc1234",
                ReviewVerdict::Approve,
                "looks fine",
                later()
            ),
            Err(Error::ClaimMismatch)
        ));
    }

    /// The accident this exists to stop: a reviewer reads commit A, the author
    /// pushes B while the review is open, and the approval of A would carry B
    /// onto the main line unread.
    #[test]
    fn an_approval_of_a_commit_the_parent_has_left_behind_is_refused() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (review_id, claim_id) = claim_review(&db, "t-1");

        // The author takes the task back and finishes it on another commit.
        set_status(&db, "t-1", TaskStatus::Blocked, later()).unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, later()).unwrap();
        let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, "t-1");
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "def5678",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();

        let refused = review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "looked good at the time",
            later(),
        )
        .expect_err("a stale approval must be refused");
        assert!(
            matches!(&refused, Error::Precondition { code, .. } if *code == "review_subject_changed"),
            "unexpected error: {refused:?}"
        );

        let review = get(&db, &review_id).unwrap();
        assert_eq!(review.status, TaskStatus::Wip, "the refusal writes nothing");
        assert!(review.review_verdict.is_none());
        assert!(review.verification.is_none());
        let parent = get(&db, "t-1").unwrap();
        assert_eq!(parent.status, TaskStatus::Done);
        assert_eq!(parent.commit_sha.as_deref(), Some("def5678"));
    }

    /// The other two halves of the same guard: the parent has to still be
    /// waiting for a verdict, and the reviewer has to name the commit the
    /// review was issued for.
    #[test]
    fn an_approval_names_its_subject_and_needs_a_parent_that_is_still_done() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (review_id, claim_id) = claim_review(&db, "t-1");

        let mismatched = review_report(
            &db,
            &claim_id,
            "def5678",
            ReviewVerdict::Approve,
            "approving something else",
            later(),
        )
        .expect_err("the body must name the commit under review");
        assert!(
            matches!(&mismatched, Error::Precondition { code, .. }
                if *code == "review_subject_mismatch"),
            "unexpected error: {mismatched:?}"
        );
        assert!(get(&db, &review_id).unwrap().review_verdict.is_none());
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Done);

        // A parent a human moved out of `done` is not waiting for a verdict.
        set_status(&db, "t-1", TaskStatus::Blocked, later()).unwrap();
        let moved = review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "read the diff",
            later(),
        )
        .expect_err("a parent that moved cannot be approved");
        assert!(
            matches!(&moved, Error::Precondition { code, .. } if *code == "review_target_moved"),
            "unexpected error: {moved:?}"
        );
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Blocked);
        assert_eq!(get(&db, &review_id).unwrap().status, TaskStatus::Wip);
        assert!(get(&db, &review_id).unwrap().verification.is_none());
    }

    /// Requesting changes is a finished review, not a failed one: the review
    /// itself is `done` and keeps the findings, and the parent goes back to the
    /// queue.
    #[test]
    fn requesting_changes_returns_the_parent_to_the_queue_with_the_findings() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (review_id, claim_id) = claim_review(&db, "t-1");

        let reported = review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::RequestChanges,
            "the guard is missing on the empty case",
            later(),
        )
        .unwrap();
        assert_eq!(
            reported.status,
            TaskStatus::Done,
            "a verdict is a finished review"
        );
        assert_eq!(reported.review_verdict, Some(ReviewVerdict::RequestChanges));
        assert_eq!(
            reported.verification.as_deref(),
            Some("the guard is missing on the empty case")
        );
        assert_eq!(get(&db, &review_id).unwrap(), reported);

        let parent = get(&db, "t-1").unwrap();
        assert_eq!(
            parent.status,
            TaskStatus::Ready,
            "the work goes back to the queue"
        );
        assert_eq!(
            parent.commit_sha.as_deref(),
            Some("abc1234"),
            "sending it back does not erase what was reported"
        );
        assert!(
            mergeable(&db).unwrap().is_empty(),
            "work sent back is not on its way to the main line"
        );
        assert!(matches!(
            issue_merge(&db, "t-1", later()),
            Err(Error::Invalid(_))
        ));
    }

    /// The catalogue gate belongs to a promotion a human asks for. A review
    /// sending work back is the continuation of work already admitted, so an
    /// archived or uncatalogued product must not strand it: the alternative is a
    /// task that can be neither approved nor returned.
    #[test]
    fn requesting_changes_does_not_consult_the_product_catalogue() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (_, claim_id) = claim_review(&db, "t-1");

        // The working copy of `a/b` left the project tree while the review ran.
        let other = Product {
            id: "z/z".into(),
            repository: "https://example.test/z/z.git".into(),
            description: String::new(),
            releases: true,
            archived: false,
        };
        let report = product::reconcile(&db, std::slice::from_ref(&other), later()).unwrap();
        assert_eq!(report.archived.len(), 1, "a/b left the tree");
        create(&db, &new_task("t-2", TaskKind::Normal, 0), now()).unwrap();
        assert!(
            matches!(
                set_status(&db, "t-2", TaskStatus::Ready, later()),
                Err(Error::Precondition {
                    code: "product_archived",
                    ..
                })
            ),
            "the ordinary promotion is still gated"
        );

        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::RequestChanges,
            "restore the clone and fix the guard",
            later(),
        )
        .expect("a review must always be able to hand the work back");
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Ready);
    }

    /// Nothing reaches the merge queue on a report alone: the reviewer's verdict
    /// is what puts it there.
    #[test]
    fn only_approved_work_is_mergeable() {
        let db = db_with_product();
        create(&db, &new_task("t-done", TaskKind::Normal, 0), now()).unwrap();
        create(&db, &new_task("t-approved", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-done");
        work_to_approved(&db, "t-approved");

        let pending: Vec<String> = pending_merges(&db)
            .unwrap()
            .into_iter()
            .filter_map(|task| task.merge_target_task_id)
            .collect();
        assert_eq!(
            pending,
            ["t-approved"],
            "a task nobody reviewed has no merge in flight"
        );
        assert!(
            matches!(issue_merge(&db, "t-done", later()), Err(Error::Invalid(_))),
            "a merge is issued against approved work only"
        );

        let merge = issued_merge(&db, "t-approved");
        let leased = claim(&db, "merger", &[TaskKind::InstantMerge], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, merge.id);
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "abc1234",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(get(&db, "t-approved").unwrap().status, TaskStatus::Merged);
    }

    /// The two completion routes do not cross. A review is finished by a
    /// verdict, and a claim on ordinary work cannot produce one.
    #[test]
    fn a_review_is_finished_by_a_verdict_and_nothing_else() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (review_id, review_claim) = claim_review(&db, "t-1");

        // The work report has no verdict to record, and no checks to gate on.
        let refused = report(
            &db,
            &review_claim,
            "abc1234",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
        .expect_err("a review is not finished by a work report");
        assert!(
            matches!(&refused, Error::Invalid(message) if message.contains("review")),
            "unexpected error: {refused:?}"
        );
        assert_eq!(get(&db, &review_id).unwrap().status, TaskStatus::Wip);
        assert!(get(&db, &review_id).unwrap().review_verdict.is_none());

        // And a verdict is not a way to finish ordinary work.
        create(&db, &new_task("t-2", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-2", TaskStatus::Ready, later()).unwrap();
        let work_claim = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap()
            .claim_id
            .expect("claim_id");
        assert!(matches!(
            review_report(
                &db,
                &work_claim,
                "abc1234",
                ReviewVerdict::Approve,
                "looks good",
                later()
            ),
            Err(Error::Invalid(_))
        ));
        assert_eq!(get(&db, "t-2").unwrap().status, TaskStatus::Wip);

        // A verdict still needs its evidence, and a review carries no checks.
        let blank = review_report(
            &db,
            &review_claim,
            "abc1234",
            ReviewVerdict::Approve,
            "   ",
            later(),
        );
        assert!(matches!(blank, Err(Error::Invalid(_))));
        assert!(get(&db, &review_id).unwrap().checks.is_empty());
        review_report(
            &db,
            &review_claim,
            "abc1234",
            ReviewVerdict::Approve,
            "read the diff",
            later(),
        )
        .expect("no checks are demanded of a reviewer");
    }

    /// A worker loop takes one role. Without a filter it still takes anything,
    /// so an existing loop keeps working.
    #[test]
    fn a_claim_may_ask_for_the_kinds_of_work_it_handles() {
        let db = db_with_product();
        create(&db, &new_task("t-work", TaskKind::Normal, 0), now()).unwrap();
        create(&db, &new_task("t-reviewed", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-reviewed");
        let review = get(&db, &review_task_id("t-reviewed")).expect("the report issues a review");
        set_status(&db, "t-work", TaskStatus::Ready, later()).unwrap();

        assert!(
            claim(&db, "luna", &[TaskKind::InstantMerge], later(), 60)
                .unwrap()
                .is_none(),
            "no work of that kind is no work, not somebody else's task"
        );

        let reviewed = claim(&db, "sol", &[TaskKind::Review], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(reviewed.id, review.id);
        assert_eq!(reviewed.kind, TaskKind::Review);

        let worked = claim(&db, "opus", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(worked.id, "t-work");

        // Unfiltered, the queue hands out whatever is next.
        create(&db, &new_task("t-any", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-any", TaskStatus::Ready, later()).unwrap();
        assert_eq!(
            claim(&db, "grok", &[], later(), 60).unwrap().unwrap().id,
            "t-any"
        );
    }

    /// The review a `done` report needs is issued by that report, in the same
    /// transaction. Nobody has to press anything: the human judgement in this
    /// workflow is the release, and a queue that waited for a person to file the
    /// review would stall behind them instead.
    #[test]
    fn a_finished_report_issues_the_review_that_reads_it() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");

        let review = get(&db, &review_task_id("t-1")).expect("the report issues the review");
        assert_eq!(review.kind, TaskKind::Review);
        assert_eq!(
            review.status,
            TaskStatus::Ready,
            "and it is claimable at once"
        );
        assert_eq!(review.review_target_task_id.as_deref(), Some("t-1"));
        assert_eq!(
            review.commit_sha.as_deref(),
            Some("abc1234"),
            "the review is issued for the commit the report carried"
        );
        assert_eq!(review.branch.as_deref(), Some("task/t-1"));
        assert_eq!(review.product_id.as_deref(), Some("a/b"));
        assert!(review.review_verdict.is_none());

        // The idempotent repeat of a report is not a second finishing, so it
        // issues nothing: the review is already there.
        let claim_id = get(&db, "t-1").unwrap().claim_id.expect("claim_id");
        report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(reviews_of(&db, "t-1"), [review.id], "one review, not two");
    }

    /// Every id of a review that reads `target_id`, oldest attempt first.
    fn reviews_of(db: &Db, target_id: &str) -> Vec<String> {
        list(db)
            .unwrap()
            .into_iter()
            .filter(|task| task.review_target_task_id.as_deref() == Some(target_id))
            .map(|task| task.id)
            .collect()
    }

    /// Every id of a merge that lands `target_id`, oldest attempt first.
    fn merges_of(db: &Db, target_id: &str) -> Vec<String> {
        list(db)
            .unwrap()
            .into_iter()
            .filter(|task| task.merge_target_task_id.as_deref() == Some(target_id))
            .map(|task| task.id)
            .collect()
    }

    /// The round trip has to keep issuing: a task handed back, reworked and
    /// reported again is in front of a reviewer once more, without anyone
    /// filing the next attempt by hand.
    #[test]
    fn a_reworked_task_is_reviewed_again_by_the_report_that_finished_it() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");

        let (first_id, claim_id) = claim_review(&db, "t-1");
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::RequestChanges,
            "the guard is missing on the empty case",
            later(),
        )
        .unwrap();
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Ready);

        let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, "t-1");
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "def5678",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();

        let second = get(&db, "review:t-1~2").expect("the next attempt is issued too");
        assert_eq!(second.status, TaskStatus::Ready);
        assert_eq!(
            second.commit_sha.as_deref(),
            Some("def5678"),
            "and it reads the reworked commit"
        );
        assert_eq!(reviews_of(&db, "t-1"), [first_id, second.id]);
    }

    /// Work that is finished again while its review is still open keeps that
    /// review, and the report goes through.
    ///
    /// The reader already exists: that review either hands the work back or is
    /// refused as stale when it tries to approve a commit the work has left
    /// behind. Refusing the report instead would throw away a worker's finished
    /// commit over a review nobody had cancelled yet.
    #[test]
    fn work_finished_again_under_an_open_review_keeps_that_one() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let open_review = get(&db, &review_task_id("t-1")).unwrap();

        set_status_by_operator(&db, "t-1", TaskStatus::Blocked, later()).unwrap();
        set_status_by_operator(&db, "t-1", TaskStatus::Ready, later()).unwrap();
        let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, "t-1");
        let redone = report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "def5678",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .expect("an open review must not cost the worker its report");
        assert_eq!(redone.status, TaskStatus::Done);
        assert_eq!(redone.commit_sha.as_deref(), Some("def5678"));
        assert_eq!(
            reviews_of(&db, "t-1"),
            std::slice::from_ref(&open_review.id),
            "the review that was already open is still the only one"
        );
        assert_eq!(
            get(&db, &open_review.id).unwrap().commit_sha.as_deref(),
            Some("abc1234"),
            "and it still reads the commit it was issued for"
        );
    }

    /// Finishing work and putting it in front of a reviewer is one act, so a
    /// review that cannot be issued at all takes the report down with it: the
    /// alternative is work that is `done`, unread, and with no way forward.
    ///
    /// The fixture is a row nothing in production writes — leased with no branch
    /// — because that is what it takes to make the issue fail for a reason other
    /// than "a review is already open", which is not a failure at all.
    #[test]
    fn a_report_that_cannot_issue_its_review_writes_nothing_at_all() {
        let db = db_with_product();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, kind, product_id, priority,
                                    claim_id, claimed_by, created_at, updated_at)
                 VALUES ('t-branchless', 'no branch', 'wip', 'normal', 'a/b', 0,
                         'claim-1', 'worker', ?1, ?1)",
                [format_z(now())],
            )?;
            Ok(())
        })
        .unwrap();

        let refused = report(
            &db,
            "claim-1",
            "abc1234",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .expect_err("work that cannot be reviewed cannot be finished");
        assert!(
            matches!(&refused, Error::Invalid(message) if message.contains("branch")),
            "unexpected error: {refused:?}"
        );

        let task = get(&db, "t-branchless").unwrap();
        assert_eq!(
            task.status,
            TaskStatus::Wip,
            "the refusal must not leave the work finished"
        );
        assert!(
            task.commit_sha.is_none(),
            "and must not have taken the commit it refused"
        );
        assert!(reviews_of(&db, "t-branchless").is_empty());
    }

    /// An approval and the merge it earns are one write. The reviewer's verdict
    /// is the last judgement before the main line, so nothing waits for a human
    /// to press "merge" afterwards.
    #[test]
    fn an_approving_verdict_issues_the_merge_in_the_same_transaction() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (_, claim_id) = claim_review(&db, "t-1");

        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "read the diff, ran the tests",
            later(),
        )
        .unwrap();

        let merge = get(&db, &merge_task_id("t-1")).expect("the approval issues the merge");
        assert_eq!(merge.kind, TaskKind::InstantMerge);
        assert_eq!(merge.status, TaskStatus::Ready);
        assert_eq!(merge.merge_target_task_id.as_deref(), Some("t-1"));
        assert_eq!(merge.commit_sha.as_deref(), Some("abc1234"));
        assert_eq!(merge.branch.as_deref(), Some("task/t-1"));
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Approved);
        assert!(
            mergeable(&db).unwrap().is_empty(),
            "the work is already spoken for by its merge"
        );
    }

    /// A verdict that asks for changes earns no merge: the work is going back
    /// to the queue, not onto the main line.
    #[test]
    fn a_verdict_that_asks_for_changes_issues_no_merge() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (_, claim_id) = claim_review(&db, "t-1");

        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::RequestChanges,
            "the guard is missing on the empty case",
            later(),
        )
        .unwrap();

        assert!(
            merges_of(&db, "t-1").is_empty(),
            "work sent back has nothing to land"
        );
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Ready);
    }

    /// Register a product so tasks of a second product have somewhere to belong.
    fn add_product(db: &Db, id: &str) {
        product::upsert(
            db,
            &Product {
                id: id.into(),
                repository: format!("https://example.test/{id}.git"),
                description: String::new(),
                releases: true,
                archived: false,
            },
            now(),
        )
        .unwrap();
    }

    /// Take `id` of `product_id` all the way to its issued merge, and answer
    /// with that merge.
    fn merge_waiting_for(db: &Db, id: &str, product_id: &str) -> Task {
        create(
            db,
            &NewTask {
                product_id: Some(product_id.into()),
                ..new_task(id, TaskKind::Normal, 0)
            },
            now(),
        )
        .unwrap();
        work_to_approved(db, id);
        issued_merge(db, id)
    }

    fn claim_merge(db: &Db, worker: &str, at: time::OffsetDateTime) -> Option<String> {
        claim(db, worker, &[TaskKind::InstantMerge], at, 60)
            .unwrap()
            .map(|task| task.id)
    }

    /// Two merges of one product cannot run at once: the second would rebase
    /// onto a main line the first has not written yet. One of them is handed
    /// out — which one is nobody's promise — and the other waits until the
    /// running one is out of the way.
    #[test]
    fn a_products_merges_are_handed_out_one_at_a_time() {
        let db = db_with_product();
        let first = merge_waiting_for(&db, "t-1", "a/b");
        let second = merge_waiting_for(&db, "t-2", "a/b");
        let both = [first.id.clone(), second.id.clone()];

        let taken = claim_merge(&db, "luna", later()).expect("one of the two is handed out");
        assert!(
            both.contains(&taken),
            "an unexpected task was claimed: {taken}"
        );
        assert_eq!(
            claim_merge(&db, "sol", later()),
            None,
            "the other merge waits while one of them is running"
        );

        // Landing the running one releases the one that waited.
        let leased = get(&db, &taken).unwrap();
        let landed = leased.merge_target_task_id.clone().expect("target");
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "merge111",
            "cargo test",
            &green(),
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(get(&db, &landed).unwrap().status, TaskStatus::Merged);

        let rest: Vec<&String> = both.iter().filter(|id| **id != taken).collect();
        assert_eq!(claim_merge(&db, "sol", later()).as_ref(), Some(rest[0]));
    }

    /// Neither of two `ready` merges may wait on the other. If they did, each
    /// would see the other and the product would never move again — the failure
    /// a strict issue order used to prevent, and that no order is needed to
    /// prevent once only a running or jammed merge holds its product.
    #[test]
    fn two_ready_merges_of_one_product_do_not_deadlock() {
        let db = db_with_product();
        let first = merge_waiting_for(&db, "t-1", "a/b");
        let second = merge_waiting_for(&db, "t-2", "a/b");
        assert_eq!(get(&db, &first.id).unwrap().status, TaskStatus::Ready);
        assert_eq!(get(&db, &second.id).unwrap().status, TaskStatus::Ready);

        assert!(
            claim_merge(&db, "luna", later()).is_some(),
            "two ready merges must not hold each other up"
        );
    }

    /// Block the head of a train the way a worker does, and answer with the row
    /// as the report left it.
    fn block_merge(db: &Db, id: &str, commit_sha: &str, reason: &str) -> Task {
        let claim_id = get(db, id).unwrap().claim_id.expect("claim_id");
        let blocked = report(
            db,
            &claim_id,
            commit_sha,
            reason,
            &[Check {
                name: "git rebase".into(),
                exit_code: 1,
            }],
            ReportOutcome::Blocked,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(blocked.status, TaskStatus::Blocked);
        blocked
    }

    /// A merge that could not be integrated stops its train, and `ready` is not
    /// the way out of it.
    ///
    /// The next merge would be rebasing onto a main line that is still waiting
    /// for this one, so nothing overtakes it. Restarting the failed attempt is
    /// refused as well: the row still carries the reason and the checks of the
    /// attempt that failed, and it is pinned to a commit whose main line has
    /// moved. Calling it off is the one press that moves the queue.
    #[test]
    fn a_blocked_merge_stops_its_train_and_ready_does_not_release_it() {
        let db = db_with_product();
        let first = merge_waiting_for(&db, "t-1", "a/b");
        let second = merge_waiting_for(&db, "t-2", "a/b");

        assert_eq!(claim_merge(&db, "luna", later()), Some(first.id.clone()));
        block_merge(&db, &first.id, "abc1234", "rebase onto main conflicts");

        assert_eq!(
            claim_merge(&db, "sol", later()),
            None,
            "a blocked merge is still in the way of the one behind it"
        );

        // Pressing `ready` would hand this very attempt back to a worker. The
        // domain refuses it, so no surface can offer it as a way past the jam.
        let refused = set_status_by_operator(&db, &first.id, TaskStatus::Ready, later());
        assert!(
            matches!(&refused, Err(Error::Invalid(message)) if message.contains("called off")),
            "restarting a blocked merge has to be refused: {refused:?}"
        );
        assert_eq!(
            get(&db, &first.id).unwrap().status,
            TaskStatus::Blocked,
            "and the refusal writes nothing"
        );
        assert_eq!(
            claim_merge(&db, "sol", later()),
            None,
            "the train is still stopped after the refused press"
        );

        set_status_by_operator(&db, &first.id, TaskStatus::Cancelled, later()).unwrap();
        assert_eq!(
            claim_merge(&db, "sol", later()),
            Some(second.id),
            "calling the blocked attempt off is what moves the train"
        );
    }

    /// The release contract read off the card a human is actually looking at:
    /// a blocked merge offers the two presses that call it off and nothing that
    /// restarts it. The list and the refusal are one rule, so an operator
    /// surface cannot offer a press the domain would then reject.
    #[test]
    fn a_blocked_merge_offers_only_the_presses_that_call_it_off() {
        let db = db_with_product();
        let merge = merge_waiting_for(&db, "t-1", "a/b");
        assert_eq!(claim_merge(&db, "luna", later()), Some(merge.id.clone()));
        let blocked = block_merge(&db, &merge.id, "abc1234", "rebase onto main conflicts");

        assert_eq!(
            available_transitions(&blocked),
            [TaskStatus::Cancelled, TaskStatus::Dropped],
            "a blocked merge is called off or nothing"
        );
        // The table still allows the edge; it is the operator rule that closes
        // it, so the control plane keeps the transition it needs.
        assert!(can_transition(TaskStatus::Blocked, TaskStatus::Ready));

        // The rule is about merge attempts and nothing else. Ordinary work that
        // stopped is picked back up by hand, exactly as before.
        create(&db, &new_task("t-stalled", TaskKind::Normal, 0), now()).unwrap();
        let stalled =
            set_status_by_operator(&db, "t-stalled", TaskStatus::Blocked, later()).unwrap();
        assert!(
            available_transitions(&stalled).contains(&TaskStatus::Ready),
            "blocked ordinary work is still restarted by hand: {:?}",
            available_transitions(&stalled)
        );
        set_status_by_operator(&db, "t-stalled", TaskStatus::Ready, later()).unwrap();
    }

    /// `dropped` is the other release, and it releases exactly as much as
    /// `cancelled`: the train moves and the target may be merged again, under a
    /// new attempt id rather than by reopening the old one.
    #[test]
    fn dropping_a_blocked_merge_frees_the_target_for_a_new_attempt() {
        let db = db_with_product();
        let first = merge_waiting_for(&db, "t-1", "a/b");
        let second = merge_waiting_for(&db, "t-2", "a/b");

        assert_eq!(claim_merge(&db, "luna", later()), Some(first.id.clone()));
        block_merge(&db, &first.id, "abc1234", "cargo test failed");

        set_status_by_operator(&db, &first.id, TaskStatus::Dropped, later()).unwrap();
        assert_eq!(
            claim_merge(&db, "sol", later()),
            Some(second.id),
            "dropping the blocked attempt moves the train too"
        );

        assert_eq!(
            mergeable(&db)
                .unwrap()
                .into_iter()
                .map(|task| task.id)
                .collect::<Vec<_>>(),
            ["t-1"],
            "the work the dropped merge would have landed is a candidate again"
        );
        let reissued = issue_merge(&db, "t-1", later()).unwrap();
        assert_eq!(
            reissued.id, "merge:t-1~2",
            "and the new attempt is a new row, not the dropped one reopened"
        );
        assert_eq!(reissued.status, TaskStatus::Ready);
        assert!(
            reissued.verification.is_none(),
            "the new attempt starts with no failure written on it"
        );
        assert_eq!(
            get(&db, &first.id).unwrap().verification.as_deref(),
            Some("cargo test failed"),
            "and the attempt that failed keeps saying why, on its own row"
        );
    }

    /// Land the merge of `target_id` the way a worker does: claim it, then
    /// report it green.
    fn land_merge(db: &Db, merge_id: &str, commit_sha: &str) -> Task {
        let claim_id = get(db, merge_id).unwrap().claim_id.expect("claim_id");
        report(
            db,
            &claim_id,
            commit_sha,
            "merged onto main",
            &[Check {
                name: "cargo test".into(),
                exit_code: 0,
            }],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap()
    }

    /// Put the merge of `t-1` into `status`, each by the route that actually
    /// gets it there rather than by writing the column.
    fn merge_into(db: &Db, merge_id: &str, status: TaskStatus) {
        match status {
            TaskStatus::Ready => {}
            TaskStatus::Wip => {
                assert_eq!(claim_merge(db, "luna", later()).as_deref(), Some(merge_id));
            }
            TaskStatus::Blocked => {
                assert_eq!(claim_merge(db, "luna", later()).as_deref(), Some(merge_id));
                block_merge(db, merge_id, "abc1234", "rebase onto main conflicts");
            }
            TaskStatus::Done => {
                assert_eq!(claim_merge(db, "luna", later()).as_deref(), Some(merge_id));
                land_merge(db, merge_id, "abc1234");
            }
            other => {
                set_status_by_operator(db, merge_id, other, later()).unwrap();
            }
        }
        assert_eq!(get(db, merge_id).unwrap().status, status);
    }

    /// The same for the review of `target_id`. `done` is reached by a verdict,
    /// which is the only thing that finishes a review.
    fn review_into(db: &Db, target_id: &str, review_id: &str, status: TaskStatus) {
        match status {
            TaskStatus::Ready => {}
            TaskStatus::Wip => {
                claim_review(db, target_id);
            }
            TaskStatus::Done => {
                let (_, claim_id) = claim_review(db, target_id);
                review_report(
                    db,
                    &claim_id,
                    "abc1234",
                    ReviewVerdict::RequestChanges,
                    "the empty case is unguarded",
                    later(),
                )
                .unwrap();
            }
            other => {
                set_status_by_operator(db, review_id, other, later()).unwrap();
            }
        }
        assert_eq!(get(db, review_id).unwrap().status, status);
    }

    /// A reconciliation window and the index that refuses a second attempt are
    /// one predicate read twice, and they have to answer together.
    ///
    /// `mergeable` offers a target exactly when `issue_merge` would accept an
    /// attempt for it, and `unreviewed` reports one exactly when `issue_review`
    /// would. Let the two spell "still holding its target" differently and the
    /// board either offers a press the control plane answers with a conflict,
    /// or hides work that really has lost its attempt and will now sit there
    /// for ever.
    ///
    /// So this asks the questions of the same row in every status an attempt
    /// can be sitting in, and pins two things at once: *which* statuses release
    /// the target — `MERGE_IS_OVER` and `REVIEW_IS_OVER`, which the windows now
    /// read rather than respell — and that the window and the index behind it
    /// never disagree about a row.
    #[test]
    fn every_reconciliation_window_agrees_with_the_index_behind_it() {
        let statuses = [
            TaskStatus::Ready,
            TaskStatus::Wip,
            TaskStatus::Blocked,
            TaskStatus::Done,
            TaskStatus::Cancelled,
            TaskStatus::Dropped,
        ];

        for status in statuses {
            let db = db_with_product();
            let merge = merge_waiting_for(&db, "t-1", "a/b");
            merge_into(&db, &merge.id, status);

            // A merge is over when it was called off, and only then: a landed
            // one keeps its target for ever, which is why `done` is not on the
            // list. `MERGE_IS_OVER` spelled as a claim about behaviour.
            let over = matches!(status, TaskStatus::Cancelled | TaskStatus::Dropped);
            let offered = mergeable(&db).unwrap().iter().any(|task| task.id == "t-1");
            assert_eq!(
                offered,
                over,
                "mergeable offers t-1 = {offered} while {} is {}",
                merge.id,
                status.as_str()
            );
            let accepted = issue_merge(&db, "t-1", later()).is_ok();
            assert_eq!(
                offered,
                accepted,
                "mergeable says {offered} and issue_merge says {accepted} \
                 while {} is {}",
                merge.id,
                status.as_str()
            );
        }

        for status in statuses {
            let db = db_with_product();
            create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
            work_to_done(&db, "t-1");
            let review_id = review_task_id("t-1");
            review_into(&db, "t-1", &review_id, status);

            // A review is over the moment it answers, and when it is called
            // off — `REVIEW_IS_OVER`. `pending_reviews` is the attempt side of
            // that one sentence.
            let over = matches!(
                status,
                TaskStatus::Done | TaskStatus::Cancelled | TaskStatus::Dropped
            );
            let listed = pending_reviews(&db)
                .unwrap()
                .iter()
                .any(|task| task.id == review_id);
            assert_eq!(
                listed,
                !over,
                "pending_reviews lists {review_id} = {listed} while it is {}",
                status.as_str()
            );

            let reported = unreviewed(&db).unwrap().iter().any(|task| task.id == "t-1");
            let accepted = issue_review(&db, "t-1", later()).is_ok();
            assert_eq!(
                reported,
                accepted,
                "unreviewed says {reported} and issue_review says {accepted} \
                 while {review_id} is {}",
                status.as_str()
            );

            // The two windows are one sentence read from both ends: while the
            // work is still sitting in `done`, exactly one of them mentions the
            // pair — either an attempt is in flight, or the work is reported as
            // having lost its reader. Both at once, or neither, is the drift.
            //
            // Once a verdict moved the work off `done` there is no pair to
            // place, so the question is asked only while it is there.
            if get(&db, "t-1").unwrap().status == TaskStatus::Done {
                assert_eq!(
                    listed,
                    !reported,
                    "pending_reviews says {listed} and unreviewed says {reported} \
                     with {review_id} in {}",
                    status.as_str()
                );
            }
        }
    }

    /// How a merge attempt ended is the worker's answer, and no press writes
    /// one.
    ///
    /// `done` and `blocked` are the two endings, and each carries evidence only
    /// `report` produces. Pressing `blocked` would stop the product's train
    /// with no reason on the row and no checks under it — a jam that cannot say
    /// what jammed it. Pressing `done` would skip `check_gate` and
    /// `land_merge_target` in one go: the attempt reads as finished while its
    /// target sits in `approved`, and it then falls out of *both* windows built
    /// to notice that — `pending_merges` stops at `done`, and `mergeable` still
    /// sees a merge row that is neither `cancelled` nor `dropped` holding the
    /// target. Approved work, never landed, on no screen at all.
    ///
    /// `wip` stays open on purpose: it is not an outcome. It says the attempt
    /// is running, invents no checks and moves no target, and the same two
    /// presses still release it.
    #[test]
    fn a_press_cannot_write_the_outcome_of_a_merge() {
        let db = db_with_product();
        let issued = merge_waiting_for(&db, "t-1", "a/b");

        assert_eq!(
            available_transitions(&issued),
            [TaskStatus::Wip, TaskStatus::Cancelled, TaskStatus::Dropped],
            "a merge waiting to be claimed offers no outcome, and keeps `wip`"
        );
        for to in [TaskStatus::Done, TaskStatus::Blocked] {
            let refused = set_status_by_operator(&db, &issued.id, to, later());
            assert!(
                matches!(&refused, Err(Error::Invalid(message))
                    if message.contains("POST /worker/report")),
                "pressing {} on an issued merge has to be refused: {refused:?}",
                to.as_str()
            );
        }
        assert_eq!(
            get(&db, &issued.id).unwrap(),
            issued,
            "and the refusals write nothing at all"
        );

        // Running, which is where `done` would have been the damaging press.
        assert_eq!(claim_merge(&db, "luna", later()), Some(issued.id.clone()));
        let running = get(&db, &issued.id).unwrap();
        assert_eq!(running.status, TaskStatus::Wip);
        assert_eq!(
            available_transitions(&running),
            [
                TaskStatus::Ready,
                TaskStatus::Cancelled,
                TaskStatus::Dropped
            ],
            "a running merge offers no outcome either"
        );
        for to in [TaskStatus::Done, TaskStatus::Blocked] {
            let refused = set_status_by_operator(&db, &issued.id, to, later());
            assert!(
                matches!(&refused, Err(Error::Invalid(message))
                    if message.contains("POST /worker/report")),
                "pressing {} on a running merge has to be refused: {refused:?}",
                to.as_str()
            );
        }
        assert_eq!(
            get(&db, &issued.id).unwrap(),
            running,
            "the merge row is untouched by the refused presses"
        );
        assert_eq!(
            get(&db, "t-1").unwrap().status,
            TaskStatus::Approved,
            "and so is the target the merge was issued for"
        );
        // The two windows that would have gone silent together.
        assert_eq!(
            pending_merges(&db)
                .unwrap()
                .into_iter()
                .map(|task| task.id)
                .collect::<Vec<_>>(),
            std::slice::from_ref(&issued.id),
            "the attempt is still in flight"
        );
        assert!(
            mergeable(&db).unwrap().is_empty(),
            "and its target is not offered a second attempt while it runs"
        );

        // The worker's own `done` still lands, gate and target together.
        let landed = land_merge(&db, &issued.id, "abc1234");
        assert_eq!(landed.status, TaskStatus::Done);
        assert_eq!(
            get(&db, "t-1").unwrap().status,
            TaskStatus::Merged,
            "the report is what moves the target"
        );

        // A landed merge is not pressed back into a jam either: `blocked` is
        // the one edge the table still allows off `done`.
        assert!(can_transition(TaskStatus::Done, TaskStatus::Blocked));
        let refused = set_status_by_operator(&db, &issued.id, TaskStatus::Blocked, later());
        assert!(
            matches!(&refused, Err(Error::Invalid(message))
                if message.contains("POST /worker/report")),
            "pressing blocked on a landed merge: {refused:?}"
        );
        assert_eq!(get(&db, &issued.id).unwrap(), landed);
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);
    }

    /// The refusal is scoped to merge attempts, and ordinary work still moves
    /// by hand exactly as it did.
    ///
    /// `done` and `blocked` are outcomes only where a report owns them. On a
    /// normal task a human is the one who says the work stopped, or finished,
    /// and `blocked -> ready` is how it is picked back up.
    #[test]
    fn ordinary_work_still_takes_done_and_blocked_by_hand() {
        let db = db_with_product();
        create(&db, &new_task("t-hand", TaskKind::Normal, 0), now()).unwrap();
        set_status_by_operator(&db, "t-hand", TaskStatus::Ready, later()).unwrap();
        set_status_by_operator(&db, "t-hand", TaskStatus::Wip, later()).unwrap();

        let running = get(&db, "t-hand").unwrap();
        for to in [TaskStatus::Done, TaskStatus::Blocked] {
            assert!(
                available_transitions(&running).contains(&to),
                "ordinary work still offers {}: {:?}",
                to.as_str(),
                available_transitions(&running)
            );
        }

        let stopped = set_status_by_operator(&db, "t-hand", TaskStatus::Blocked, later()).unwrap();
        assert_eq!(stopped.status, TaskStatus::Blocked);
        let restarted = set_status_by_operator(&db, "t-hand", TaskStatus::Ready, later()).unwrap();
        assert_eq!(
            restarted.status,
            TaskStatus::Ready,
            "blocked ordinary work is restarted by hand"
        );
        set_status_by_operator(&db, "t-hand", TaskStatus::Wip, later()).unwrap();
        let finished = set_status_by_operator(&db, "t-hand", TaskStatus::Done, later()).unwrap();
        assert_eq!(finished.status, TaskStatus::Done);
    }

    /// `done_at` is the first time a task reached `done`, not the latest
    /// status change: it is written once, on the transition into `done`, and
    /// every later transition — approval, landing, release — leaves it alone.
    #[test]
    fn done_at_is_recorded_once_and_unmoved_by_approval_merge_or_release() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();

        // The worker-report path writes `done_at`, not a pressed status.
        let leased = claim(&db, "worker", &[TaskKind::Normal], now(), 60)
            .unwrap()
            .unwrap();
        report(
            &db,
            &leased.claim_id.clone().unwrap(),
            "abc1234",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            now(),
        )
        .unwrap();
        let finished = get(&db, "t-1").unwrap();
        assert_eq!(finished.done_at.as_deref(), Some(format_z(now()).as_str()));

        // The review's own approve write moves the target `done -> approved`
        // through a dedicated UPDATE, not the one `done_at` is guarded on.
        let (_, review_claim) = claim_review(&db, "t-1");
        let approved = review_report(
            &db,
            &review_claim,
            "abc1234",
            ReviewVerdict::Approve,
            "looks good",
            later(),
        )
        .unwrap();
        assert_eq!(approved.status, TaskStatus::Done, "the review, not t-1");
        let approved = get(&db, "t-1").unwrap();
        assert_eq!(
            approved.status,
            TaskStatus::Approved,
            "the approval promoted t-1"
        );
        assert_eq!(
            approved.done_at, finished.done_at,
            "approval does not move the completion time"
        );

        // Landing goes through `land_merge_target`'s own UPDATE.
        let merge_leased = claim(&db, "builder", &[TaskKind::InstantMerge], even_later(), 60)
            .unwrap()
            .unwrap();
        report(
            &db,
            &merge_leased.claim_id.clone().unwrap(),
            "abc1234",
            "landed",
            &[Check {
                name: "cargo test".into(),
                exit_code: 0,
            }],
            ReportOutcome::Done,
            None,
            even_later(),
        )
        .unwrap();
        let merged = get(&db, "t-1").unwrap();
        assert_eq!(merged.status, TaskStatus::Merged);
        assert_eq!(
            merged.done_at, finished.done_at,
            "landing does not move it either"
        );

        // Release goes through the release task's own report UPDATE.
        let release = claim_release(&db);
        ship(&db, &release, Some("v1.0.0")).unwrap();
        let released = get(&db, "t-1").unwrap();
        assert_eq!(released.status, TaskStatus::Released);
        assert_eq!(released.done_at, finished.done_at, "nor does release");
    }

    /// The worker-report path writes `done_at` the same way the operator press
    /// does, and a task sent back and finished a second time keeps the first
    /// timestamp — "first" is the whole point of the column.
    #[test]
    fn done_at_survives_a_second_pass_through_done() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let first = get(&db, "t-1").unwrap();
        assert_eq!(first.done_at.as_deref(), Some(format_z(now()).as_str()));

        set_status_by_operator(&db, "t-1", TaskStatus::Blocked, later()).unwrap();
        set_status_by_operator(&db, "t-1", TaskStatus::Ready, later()).unwrap();
        set_status_by_operator(&db, "t-1", TaskStatus::Wip, later()).unwrap();
        let second = set_status_by_operator(&db, "t-1", TaskStatus::Done, even_later()).unwrap();
        assert_eq!(
            second.done_at, first.done_at,
            "done_at is the first time this task reached done, not the latest"
        );
    }

    /// The done screen reads completed `normal` work only, newest first, and
    /// never a review or merge subtask — even once that subtask is itself
    /// `done` or `merged`.
    #[test]
    fn list_done_orders_completed_normal_work_by_completion_time_and_excludes_subtasks() {
        let db = db_with_product();

        create(&db, &new_task("t-early", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-early", TaskStatus::Ready, now()).unwrap();
        set_status(&db, "t-early", TaskStatus::Wip, now()).unwrap();
        set_status(&db, "t-early", TaskStatus::Done, now()).unwrap();

        // t-mid finishes later, through the real worker-report -> review ->
        // approve -> merge pipeline, so its review and merge subtasks exist
        // and have to stay out of the done list even though both are
        // themselves `done` or `merged`.
        create(&db, &new_task("t-mid", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-mid", TaskStatus::Ready, now()).unwrap();
        let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        report(
            &db,
            &leased.claim_id.clone().unwrap(),
            "abc1234",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        let (_, review_claim) = claim_review(&db, "t-mid");
        review_report(
            &db,
            &review_claim,
            "abc1234",
            ReviewVerdict::Approve,
            "looks good",
            later(),
        )
        .unwrap();
        let merge_leased = claim(&db, "builder", &[TaskKind::InstantMerge], later(), 60)
            .unwrap()
            .unwrap();
        report(
            &db,
            &merge_leased.claim_id.clone().unwrap(),
            "abc1234",
            "landed",
            &[Check {
                name: "cargo test".into(),
                exit_code: 0,
            }],
            ReportOutcome::Done,
            None,
            even_later(),
        )
        .unwrap();

        // t-approved finishes last and stops at `approved`: reviewed but not
        // yet landed, still one of the statuses the done screen shows.
        create(&db, &new_task("t-approved", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-approved", TaskStatus::Ready, now()).unwrap();
        let leased = claim(&db, "worker", &[TaskKind::Normal], even_later(), 60)
            .unwrap()
            .unwrap();
        report(
            &db,
            &leased.claim_id.clone().unwrap(),
            "ccc3333",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            even_later(),
        )
        .unwrap();
        let (_, review_claim) = claim_review(&db, "t-approved");
        review_report(
            &db,
            &review_claim,
            "ccc3333",
            ReviewVerdict::Approve,
            "looks good",
            even_later(),
        )
        .unwrap();

        create(&db, &new_task("t-open", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-open", TaskStatus::Ready, now()).unwrap();

        let rows = list_done(&db).unwrap();
        let ids: Vec<String> = rows.iter().map(|task| task.id.clone()).collect();
        assert_eq!(
            ids,
            ["t-approved", "t-mid", "t-early"],
            "most recently completed first; open work, reviews, and merges are absent"
        );
        assert_eq!(rows[0].status, TaskStatus::Approved);
        assert_eq!(rows[1].status, TaskStatus::Merged);
        assert_eq!(rows[2].status, TaskStatus::Done);
    }

    /// A train is one product's. Another product's merges are rebasing onto
    /// another main line, and run beside it.
    #[test]
    fn a_stalled_train_does_not_hold_up_another_product() {
        let db = db_with_product();
        add_product(&db, "c/d");
        let held = merge_waiting_for(&db, "t-1", "a/b");
        merge_waiting_for(&db, "t-2", "a/b");
        let elsewhere = merge_waiting_for(&db, "t-3", "c/d");

        assert_eq!(claim_merge(&db, "luna", later()), Some(held.id.clone()));
        let claim_id = get(&db, &held.id).unwrap().claim_id.expect("claim_id");
        report(
            &db,
            &claim_id,
            "abc1234",
            "rebase onto main conflicts",
            &[],
            ReportOutcome::Blocked,
            None,
            later(),
        )
        .unwrap();

        assert_eq!(
            claim_merge(&db, "sol", later()),
            Some(elsewhere.id),
            "another product's train runs while this one is stopped"
        );
    }

    /// An expired lease is the running merge being handed to somebody else,
    /// never the one waiting behind it moving up. The running row is `wip`, so
    /// it holds the rest of its product back; it is exempt from its own test, so
    /// it is the one that can be taken again.
    #[test]
    fn an_expired_merge_lease_is_retaken_not_overtaken() {
        let db = db_with_product();
        let one = merge_waiting_for(&db, "t-a", "a/b");
        let other = merge_waiting_for(&db, "t-b", "a/b");

        let running = claim_merge(&db, "luna", later()).expect("one of the two runs");
        let waiting = [one.id, other.id]
            .into_iter()
            .find(|id| *id != running)
            .expect("the other one waits");

        let expired = later() + time::Duration::seconds(61);
        let retaken = claim_merge(&db, "sol", expired).expect("the stalled merge comes back");
        assert_eq!(
            retaken, running,
            "the stalled merge is retaken, not overtaken"
        );
        assert_ne!(retaken, waiting);
    }

    /// A merge that could not be integrated has to leave a record. Rolling the
    /// report back would put the merge straight back in the queue with nothing
    /// anywhere saying why it failed, and the next worker would walk into the
    /// same conflict.
    #[test]
    fn a_blocked_merge_report_keeps_the_reason_and_the_checks() {
        let db = db_with_product();
        let merge = merge_waiting_for(&db, "t-1", "a/b");
        assert_eq!(claim_merge(&db, "luna", later()), Some(merge.id.clone()));
        let claim_id = get(&db, &merge.id).unwrap().claim_id.expect("claim_id");
        let red = vec![Check {
            name: "cargo test".into(),
            exit_code: 101,
        }];

        let blocked = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test failed on the rebased branch",
            &red,
            ReportOutcome::Blocked,
            None,
            later(),
        )
        .expect("a worker reporting a red check is not itself an error");
        assert_eq!(blocked.status, TaskStatus::Blocked);
        assert_eq!(
            blocked.verification.as_deref(),
            Some("cargo test failed on the rebased branch"),
            "the reason is what a human reads off the merge"
        );
        assert_eq!(blocked.checks, red, "and the evidence is kept with it");
        assert_eq!(
            blocked.commit_sha.as_deref(),
            Some("abc1234"),
            "a blocked merge keeps the subject it was issued for"
        );
        assert_eq!(
            get(&db, "t-1").unwrap().status,
            TaskStatus::Approved,
            "nothing landed, so the target does not move"
        );

        // A worker that did not hear the answer says the same thing again.
        let again = report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test failed on the rebased branch",
            &red,
            ReportOutcome::Blocked,
            None,
            later(),
        )
        .unwrap();
        assert_eq!(again, blocked);

        // A different reason on a merge that is already blocked is a second
        // answer, not a repeat, and the first one stands.
        assert!(matches!(
            report(
                &db,
                &claim_id,
                "abc1234",
                "actually it was a rebase conflict",
                &red,
                ReportOutcome::Blocked,
                None,
                later(),
            ),
            Err(Error::Invalid(_))
        ));
        // And a blocked merge cannot be turned into a landed one by reporting
        // success over it: a human calls it off and the approval issues another.
        assert!(matches!(
            report(
                &db,
                &claim_id,
                "abc1234",
                "cargo test",
                &green(),
                ReportOutcome::Done,
                None,
                later(),
            ),
            Err(Error::Invalid(_))
        ));
        assert_eq!(get(&db, &merge.id).unwrap(), blocked);
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Approved);
    }

    /// The evidence rule guards success only. A worker saying "this went red" is
    /// reporting the failure, not claiming it as a pass, so the checks that would
    /// refuse a landing are exactly what a blocked report is there to carry —
    /// and a report with no checks at all is still a report of a conflict.
    #[test]
    fn the_check_gate_guards_landing_and_not_the_report_of_a_failure() {
        let db = db_with_product();
        let merge = merge_waiting_for(&db, "t-1", "a/b");
        assert_eq!(claim_merge(&db, "luna", later()), Some(merge.id.clone()));
        let claim_id = get(&db, &merge.id).unwrap().claim_id.expect("claim_id");

        assert!(
            matches!(
                report(
                    &db,
                    &claim_id,
                    "abc1234",
                    "cargo test",
                    &[],
                    ReportOutcome::Done,
                    None,
                    later(),
                ),
                Err(Error::Invalid(_))
            ),
            "landing still needs evidence"
        );
        let blocked = report(
            &db,
            &claim_id,
            "abc1234",
            "rebase onto main conflicts in src/task.rs",
            &[],
            ReportOutcome::Blocked,
            None,
            later(),
        )
        .expect("a conflict is reported without checks");
        assert_eq!(blocked.status, TaskStatus::Blocked);
        assert!(blocked.checks.is_empty());
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Approved);
    }

    /// What the admin screen reads. `pending_reviews` and `pending_merges` are
    /// what is in flight; `unreviewed` is the alarm — work that finished and
    /// lost its reader, which the automatic issuing means should never happen.
    #[test]
    fn the_control_plane_lists_open_reviews_and_work_that_lost_its_reader() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        create(&db, &new_task("t-2", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");

        let ids =
            |tasks: Vec<Task>| -> Vec<String> { tasks.into_iter().map(|task| task.id).collect() };
        assert_eq!(ids(pending_reviews(&db).unwrap()), ["review:t-1"]);
        assert!(
            unreviewed(&db).unwrap().is_empty(),
            "work that finished is already being read"
        );

        // A cancelled attempt is the way that reader is lost, and the window is
        // what makes the silence visible: `done` goes nowhere without a verdict.
        set_status_by_operator(&db, "review:t-1", TaskStatus::Cancelled, later()).unwrap();
        assert!(pending_reviews(&db).unwrap().is_empty());
        assert_eq!(ids(unreviewed(&db).unwrap()), ["t-1"]);

        // Issuing one by hand is the remedy, and closes the window again.
        let second = issue_review(&db, "t-1", later()).unwrap();
        assert_eq!(
            ids(pending_reviews(&db).unwrap()),
            std::slice::from_ref(&second.id)
        );
        assert!(unreviewed(&db).unwrap().is_empty());

        // Work still in flight is in neither list, and neither is a merge.
        set_status(&db, "t-2", TaskStatus::Ready, later()).unwrap();
        assert!(unreviewed(&db).unwrap().is_empty());

        let (_, claim_id) = claim_review(&db, "t-1");
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "read the diff",
            later(),
        )
        .unwrap();
        assert!(
            pending_reviews(&db).unwrap().is_empty(),
            "an answered review is not pending"
        );
        assert!(unreviewed(&db).unwrap().is_empty());
        assert_eq!(ids(pending_merges(&db).unwrap()), ["merge:t-1"]);
    }

    /// A worker whose change was sent back has to be able to read why, and the
    /// answer has to be the review's own row rather than a second copy of it.
    #[test]
    fn the_parent_reads_the_verdict_and_findings_of_its_latest_review() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        assert!(
            latest_review(&db, "t-1").unwrap().is_none(),
            "an unreviewed task has no verdict"
        );

        let (first_id, claim_id) = claim_review(&db, "t-1");
        assert!(
            latest_review(&db, "t-1").unwrap().is_none(),
            "an open review has not said anything yet"
        );
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::RequestChanges,
            "the guard is missing on the empty case",
            later(),
        )
        .unwrap();

        let outcome = latest_review(&db, "t-1").unwrap().expect("a verdict");
        assert_eq!(outcome.review_task_id, first_id);
        assert_eq!(outcome.verdict, ReviewVerdict::RequestChanges);
        assert_eq!(
            outcome.findings.as_deref(),
            Some("the guard is missing on the empty case")
        );
        assert_eq!(outcome.subject_commit_sha.as_deref(), Some("abc1234"));

        // The rework is reviewed again, and the newest verdict is the answer.
        let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        report(
            &db,
            &leased.claim_id.expect("claim_id"),
            "def5678",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        let (second_id, claim_id) = claim_review(&db, "t-1");
        review_report(
            &db,
            &claim_id,
            "def5678",
            ReviewVerdict::Approve,
            "the guard is there now",
            later(),
        )
        .unwrap();

        let outcome = latest_review(&db, "t-1").unwrap().expect("a verdict");
        assert_ne!(second_id, first_id);
        assert_eq!(outcome.review_task_id, second_id);
        assert_eq!(outcome.verdict, ReviewVerdict::Approve);
        assert_eq!(outcome.findings.as_deref(), Some("the guard is there now"));
        assert_eq!(outcome.subject_commit_sha.as_deref(), Some("def5678"));
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Approved);
    }

    // --- depends_on ---

    fn dependent(db: &Db, id: &str, depends_on: &str) -> Task {
        create(
            db,
            &NewTask {
                depends_on: Some(depends_on.into()),
                ..new_task(id, TaskKind::Normal, 0)
            },
            now(),
        )
        .unwrap()
    }

    /// A → B → C: the landing of A promotes B and nothing else; C waits for
    /// B's landing. A dependency is one link, and a chain is how several are
    /// expressed.
    #[test]
    fn a_landing_promotes_the_draft_that_waited_for_it_and_only_that_one() {
        let db = db_with_product();
        create(&db, &new_task("t-a", TaskKind::Normal, 0), now()).unwrap();
        let b = dependent(&db, "t-b", "t-a");
        assert_eq!(b.depends_on.as_deref(), Some("t-a"));
        dependent(&db, "t-c", "t-b");
        assert_eq!(
            dependency_status(&db, &b).unwrap(),
            Some(TaskStatus::Draft),
            "the card says what the dependency is doing"
        );

        work_to_approved(&db, "t-a");
        merge_into(&db, &issued_merge(&db, "t-a").id, TaskStatus::Done);
        assert_eq!(get(&db, "t-a").unwrap().status, TaskStatus::Merged);

        assert_eq!(
            get(&db, "t-b").unwrap().status,
            TaskStatus::Ready,
            "the landing promoted the task that waited for it"
        );
        assert_eq!(
            get(&db, "t-c").unwrap().status,
            TaskStatus::Draft,
            "the chain waits one link at a time"
        );
        assert_eq!(
            dependency_status(&db, &get(&db, "t-b").unwrap()).unwrap(),
            None
        );

        // t-b is already `ready` (the landing put it there), so it is driven
        // from the claim on.
        let leased = claim(&db, "worker", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, "t-b");
        report(
            &db,
            &leased.claim_id.unwrap(),
            "abc1234",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            later(),
        )
        .unwrap();
        let (_, claim_id) = claim_review(&db, "t-b");
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "read it",
            later(),
        )
        .unwrap();
        merge_into(&db, &issued_merge(&db, "t-b").id, TaskStatus::Done);
        assert_eq!(get(&db, "t-c").unwrap().status, TaskStatus::Ready);
    }

    /// `released` is a landing too: a product that does not release ends at
    /// the merge, and its dependants are promoted from there.
    #[test]
    fn a_release_promotes_the_draft_that_waited_for_it() {
        let db = db_with_product();
        product::upsert(
            &db,
            &Product {
                id: "c/d".into(),
                repository: "https://example.test/c/d.git".into(),
                description: String::new(),
                releases: false,
                archived: false,
            },
            now(),
        )
        .unwrap();
        create(
            &db,
            &NewTask {
                product_id: Some("c/d".into()),
                ..new_task("t-a", TaskKind::Normal, 0)
            },
            now(),
        )
        .unwrap();
        dependent(&db, "t-b", "t-a");

        work_to_approved(&db, "t-a");
        let merge = issued_merge(&db, "t-a");
        assert_eq!(
            claim_merge(&db, "luna", later()).as_deref(),
            Some(merge.id.as_str())
        );
        land_merge(&db, &merge.id, "abc1234");
        assert_eq!(get(&db, "t-a").unwrap().status, TaskStatus::Released);
        assert_eq!(get(&db, "t-b").unwrap().status, TaskStatus::Ready);

        // The control plane granting `released` directly promotes the same way.
        create(&db, &new_task("t-x", TaskKind::Normal, 0), now()).unwrap();
        dependent(&db, "t-y", "t-x");
        for to in [
            TaskStatus::Ready,
            TaskStatus::Wip,
            TaskStatus::Done,
            TaskStatus::Approved,
            TaskStatus::Merged,
        ] {
            set_status(&db, "t-x", to, now()).unwrap();
        }
        assert_eq!(get(&db, "t-y").unwrap().status, TaskStatus::Ready);
    }

    /// The order belongs to the dependency, not to a hand: `ready` is refused
    /// while the dependency has not landed, and the refusal names it.
    #[test]
    fn a_pressed_ready_is_refused_while_the_dependency_has_not_landed() {
        let db = db_with_product();
        create(&db, &new_task("t-a", TaskKind::Normal, 0), now()).unwrap();
        dependent(&db, "t-b", "t-a");
        set_status(&db, "t-a", TaskStatus::Ready, now()).unwrap();

        let refused = set_status_by_operator(&db, "t-b", TaskStatus::Ready, now()).unwrap_err();
        match refused {
            Error::Precondition { code, message } => {
                assert_eq!(code, "dependency_pending");
                assert!(
                    message.contains("t-a") && message.contains("ready"),
                    "{message}"
                );
            }
            other => panic!("expected a precondition, got {other:?}"),
        }
        assert_eq!(get(&db, "t-b").unwrap().status, TaskStatus::Draft);

        // Clearing the dependency is the way past the order.
        update(
            &db,
            "t-b",
            &TaskPatch {
                depends_on: Some(None),
                ..TaskPatch::default()
            },
            later(),
        )
        .unwrap();
        assert!(get(&db, "t-b").unwrap().depends_on.is_none());
        set_status_by_operator(&db, "t-b", TaskStatus::Ready, later()).unwrap();
    }

    #[test]
    fn a_dependency_on_itself_an_unknown_task_or_a_cycle_is_refused() {
        let db = db_with_product();
        create(&db, &new_task("t-a", TaskKind::Normal, 0), now()).unwrap();
        for (id, dependency) in [("t-self", "t-self"), ("t-ghost", "no-such-task")] {
            let refused = create(
                &db,
                &NewTask {
                    depends_on: Some(dependency.into()),
                    ..new_task(id, TaskKind::Normal, 0)
                },
                now(),
            )
            .unwrap_err();
            assert!(matches!(refused, Error::Invalid(_)), "{id}: {refused:?}");
            assert!(
                matches!(get(&db, id), Err(Error::NotFound)),
                "nothing was written"
            );
        }

        dependent(&db, "t-b", "t-a");
        let cycle = update(
            &db,
            "t-a",
            &TaskPatch {
                depends_on: Some(Some("t-b".into())),
                ..TaskPatch::default()
            },
            now(),
        )
        .unwrap_err();
        assert!(matches!(cycle, Error::Invalid(_)), "{cycle:?}");
        assert!(get(&db, "t-a").unwrap().depends_on.is_none());

        set_status_by_operator(&db, "t-a", TaskStatus::Cancelled, now()).unwrap();
        let called_off = create(
            &db,
            &NewTask {
                depends_on: Some("t-a".into()),
                ..new_task("t-late", TaskKind::Normal, 0)
            },
            now(),
        )
        .unwrap_err();
        assert!(matches!(called_off, Error::Invalid(_)), "{called_off:?}");
    }

    /// A dependency called off after the fact blocks what waited for it,
    /// with the reason on the row, rather than leaving it `draft` for ever.
    #[test]
    fn a_dependency_that_is_cancelled_blocks_its_dependants_with_the_reason() {
        let db = db_with_product();
        create(&db, &new_task("t-a", TaskKind::Normal, 0), now()).unwrap();
        dependent(&db, "t-b", "t-a");

        set_status_by_operator(&db, "t-a", TaskStatus::Cancelled, later()).unwrap();

        let blocked = get(&db, "t-b").unwrap();
        assert_eq!(blocked.status, TaskStatus::Blocked);
        assert!(
            blocked
                .verification
                .as_deref()
                .unwrap_or("")
                .contains("t-a"),
            "{blocked:?}"
        );
    }

    /// The promotion goes through the catalogue gate like a pressed `ready`,
    /// and a task it cannot promote says why instead of staying quietly draft.
    #[test]
    fn a_dependant_whose_product_is_not_catalogued_stays_draft_and_says_why() {
        let db = db_with_product();
        create(&db, &new_task("t-a", TaskKind::Normal, 0), now()).unwrap();
        create(
            &db,
            &NewTask {
                product_id: Some("x/y".into()),
                depends_on: Some("t-a".into()),
                ..new_task("t-b", TaskKind::Normal, 0)
            },
            now(),
        )
        .unwrap();

        work_to_approved(&db, "t-a");
        merge_into(&db, &issued_merge(&db, "t-a").id, TaskStatus::Done);

        let left = get(&db, "t-b").unwrap();
        assert_eq!(left.status, TaskStatus::Draft);
        assert!(
            left.verification
                .as_deref()
                .unwrap_or("")
                .contains("not in the product catalogue"),
            "{left:?}"
        );
    }
    // --- handing a claim back ---

    /// A live claim handed back puts the task where the claim found it, with
    /// the reason on the row, and the next claim takes it again.
    #[test]
    fn a_live_claim_handed_back_returns_the_task_to_ready_with_the_reason() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        set_status(&db, "t-1", TaskStatus::Ready, now()).unwrap();
        let leased = claim(&db, "worker-a", &[TaskKind::Normal], now(), 60)
            .unwrap()
            .unwrap();
        let claim_id = leased.claim_id.clone().unwrap();

        let back = release_claim(&db, &claim_id, "self-update", later()).unwrap();
        assert_eq!(back.status, TaskStatus::Ready);
        assert!(back.claim_id.is_none() && back.claimed_by.is_none());
        assert!(back.claimed_at.is_none() && back.claim_expires_at.is_none());
        assert_eq!(
            back.verification.as_deref(),
            Some("claim released by worker-a: self-update")
        );
        assert_eq!(back.branch.as_deref(), Some("task/t-1"), "the branch stays");

        // Handing it back twice is a conflict, and the task is not touched.
        assert!(matches!(
            release_claim(&db, &claim_id, "shutdown", later()),
            Err(Error::Precondition {
                code: "claim_not_live",
                ..
            })
        ));
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Ready);

        // The next claim takes it, under a fresh claim_id, and a second
        // hand-back appends to what is already on the row.
        let again = claim(&db, "worker-b", &[TaskKind::Normal], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(again.id, "t-1");
        assert_ne!(again.claim_id, Some(claim_id));
        let back =
            release_claim(&db, again.claim_id.as_deref().unwrap(), "gave-up", later()).unwrap();
        assert_eq!(
            back.verification.as_deref(),
            Some("claim released by worker-a: self-update\nclaim released by worker-b: gave-up")
        );
    }

    /// Only a live lease can be handed back: an expired one, a reported one,
    /// and an unknown id are all refused with `claim_not_live`.
    #[test]
    fn an_expired_reported_or_unknown_claim_cannot_be_handed_back() {
        let db = db_with_product();
        for id in ["t-expired", "t-reported"] {
            create(&db, &new_task(id, TaskKind::Normal, 0), now()).unwrap();
            set_status(&db, id, TaskStatus::Ready, now()).unwrap();
        }
        let expired = claim(&db, "worker", &[TaskKind::Normal], now(), 1)
            .unwrap()
            .unwrap();
        assert_eq!(expired.id, "t-expired");
        let past = now() + time::Duration::seconds(5);
        assert!(matches!(
            release_claim(&db, expired.claim_id.as_deref().unwrap(), "shutdown", past),
            Err(Error::Precondition {
                code: "claim_not_live",
                ..
            })
        ));
        assert_eq!(get(&db, "t-expired").unwrap().status, TaskStatus::Wip);

        let reported = claim(&db, "worker", &[TaskKind::Normal], now(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(reported.id, "t-reported");
        let claim_id = reported.claim_id.clone().unwrap();
        report(
            &db,
            &claim_id,
            "abc1234",
            "cargo test",
            &[],
            ReportOutcome::Done,
            None,
            now(),
        )
        .unwrap();
        assert!(matches!(
            release_claim(&db, &claim_id, "shutdown", later()),
            Err(Error::Precondition {
                code: "claim_not_live",
                ..
            })
        ));
        assert_eq!(get(&db, "t-reported").unwrap().status, TaskStatus::Done);

        assert!(matches!(
            release_claim(&db, "no-such-claim", "shutdown", later()),
            Err(Error::Precondition {
                code: "claim_not_live",
                ..
            })
        ));
        assert!(matches!(
            release_claim(&db, &claim_id, "  ", later()),
            Err(Error::Invalid(_))
        ));
    }

    /// A review's claim can be handed back too, and the attempt is not
    /// counted: the attempt is over only when a verdict is written.
    #[test]
    fn a_review_claim_handed_back_keeps_its_attempt_number() {
        let db = db_with_product();
        create(&db, &new_task("t-1", TaskKind::Normal, 0), now()).unwrap();
        work_to_done(&db, "t-1");
        let (review_id, claim_id) = claim_review(&db, "t-1");
        let attempt_of = |id: &str| -> i64 {
            db.with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT review_attempt FROM tasks WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )?)
            })
            .unwrap()
        };
        let attempt_before = attempt_of(&review_id);

        let back = release_claim(&db, &claim_id, "shutdown", later()).unwrap();
        assert_eq!(back.id, review_id);
        assert_eq!(back.status, TaskStatus::Ready);
        assert!(back.review_verdict.is_none());
        assert_eq!(attempt_of(&review_id), attempt_before);

        // The same review is claimed again and answered as usual.
        let (again, claim_id) = claim_review(&db, "t-1");
        assert_eq!(again, review_id);
        review_report(
            &db,
            &claim_id,
            "abc1234",
            ReviewVerdict::Approve,
            "read it",
            later(),
        )
        .unwrap();
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Approved);
    }
}
