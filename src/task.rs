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
                       review_target_task_id, review_verdict";

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
}

impl TaskKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::InstantMerge => "instant:merge",
            Self::Review => "review",
        }
    }

    /// The control plane route that issues this kind, for a refusal that can
    /// say where the task should have come from. Ordinary work has none.
    fn issued_by(self) -> Option<&'static str> {
        match self {
            Self::Normal => None,
            Self::InstantMerge => Some("POST /api/merges"),
            Self::Review => Some("POST /api/reviews"),
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "normal" => Ok(Self::Normal),
            "instant:merge" => Ok(Self::InstantMerge),
            "review" => Ok(Self::Review),
            other => Err(Error::Invalid(format!("invalid kind: {other}"))),
        }
    }
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
    //     'dropped')` — so an attempt that is over would stand in the way of the
    //     next review of the same target;
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
    // 'cancelled', 'dropped')` — it would free the target for the next review
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
    None
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
        // The catalogue is derived from the project tree wherever one is
        // configured, so "add the product" is the wrong instruction there: no
        // clone sits at this id, and a row typed in by hand would be archived by
        // the next walk. The remedy is the tree, with the API named only for the
        // deployment that has no tree at all.
        Err(Error::NotFound) => Err(Error::Precondition {
            code: "product_not_catalogued",
            message: format!(
                "product '{product_id}' is not in the product catalogue, \
                 so task {} cannot become ready; the catalogue follows the project tree, \
                 so put a clone at {product_id} or correct the product_id \
                 (with no project tree configured, add it with PUT /api/products/{product_id})",
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
    if worker.trim().is_empty() {
        return Err(Error::Invalid("worker is required".into()));
    }
    let ttl = i64::try_from(ttl_secs).unwrap_or(i64::MAX);
    let claimed_at = format_z(now);
    let claim_expires_at = format_z(now + time::Duration::seconds(ttl));
    let select_sql = format!(
        "SELECT {COLUMNS} FROM tasks WHERE {}{}
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
    Ok(())
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
    db.with_tx(|tx| {
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
        .map_err(|err| {
            attempt_conflict(
                err,
                format!("task {target_id} already has a merge in flight"),
            )
        })?;
        read(tx, &id)
    })
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
    db.with_tx(|tx| {
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
                                commit_sha, review_target_task_id, review_attempt,
                                created_at, updated_at)
             VALUES (?1, ?2, '', 'ready', 'review', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            rusqlite::params![
                id,
                format!("review {target_id}: {}", target.title),
                target.product_id,
                target.priority,
                branch,
                commit_sha,
                target_id,
                attempt,
                stamp,
            ],
        )
        .map_err(|err| {
            attempt_conflict(
                err,
                format!("task {target_id} already has a review in flight"),
            )
        })?;
        read(tx, &id)
    })
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
        ALL_STATUSES, Check, NewTask, Releasable, ReviewVerdict, TaskKind, TaskPatch, TaskStatus,
        available_transitions, can_transition, claim, create, get, issue_merge, issue_review,
        latest_review, list, list_active, list_by_status, merge_task_id, mergeable, pending_merges,
        releasable, release_product, report, review_report, review_task_id, set_status,
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
        assert!(
            claim(&db, "worker-c", &[], far_future, 60)
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

    /// Take `id` from draft to done the way a worker does.
    fn work_to_done(db: &Db, id: &str) {
        set_status(db, id, TaskStatus::Ready, now()).unwrap();
        let leased = claim(db, "worker", &[], now(), 60).unwrap().unwrap();
        assert_eq!(leased.id, id);
        let claim_id = leased.claim_id.expect("claim_id");
        report(db, &claim_id, "abc1234", "cargo test", &[], now()).unwrap();
    }

    /// Issue a review for `target_id` and claim it the way a reviewer does.
    fn claim_review(db: &Db, target_id: &str) -> (String, String) {
        let review = issue_review(db, target_id, later()).unwrap();
        let leased = claim(db, "reviewer", &[TaskKind::Review], later(), 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, review.id, "the reviewer must get the review");
        (review.id, leased.claim_id.expect("claim_id"))
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
        work_to_approved(&db, "t-1");
        let merge = issue_merge(&db, "t-1", later()).unwrap();
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
                    report(&db, &claim_id, "abc1234", "cargo test", &checks, later()),
                    Err(Error::Invalid(_))
                ),
                "{checks:?} must not land"
            );
            assert_eq!(get(&db, &merge.id).unwrap().status, TaskStatus::Wip);
            assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Approved);
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
        work_to_approved(&db, "t-1");
        let merge = issue_merge(&db, "t-1", later()).unwrap();
        let claim_id = claim(&db, "worker", &[], later(), 60)
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
        work_to_approved(&db, "t-1");

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
        work_to_approved(&db, "t-1");
        let merge = issue_merge(&db, "t-1", later()).unwrap();
        let leased = claim(&db, "worker", &[], later(), 60).unwrap().unwrap();
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
        let claim_id = claim(&db, "worker", &[], now(), 60)
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
                archived: false,
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
                TaskStatus::Approved,
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
        work_to_approved(&db, "t-1");
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

        let leased = claim(&db, "worker", &[], now(), 60).unwrap().unwrap();
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
        let review = issue_review(&db, "t-1", later()).unwrap();

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
        let merge = issue_merge(&db, "t-1", later()).unwrap();
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

        let review = issue_review(&db, "t-1", later()).unwrap();
        assert_eq!(review.id, review_task_id("t-1"));
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
            later(),
        )
        .unwrap();

        let second = issue_review(&db, "t-1", later()).unwrap();
        assert_eq!(second.id, format!("{first_id}~2"));
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

        let ids: Vec<String> = mergeable(&db).unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            ["t-approved"],
            "a task nobody reviewed is not a merge candidate"
        );
        assert!(
            matches!(issue_merge(&db, "t-done", later()), Err(Error::Invalid(_))),
            "a merge is issued against approved work only"
        );

        let merge = issue_merge(&db, "t-approved", later()).unwrap();
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
        let review = issue_review(&db, "t-reviewed", later()).unwrap();
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
}
