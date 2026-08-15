use rusqlite::{Connection, Row, ToSql};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::clock::format_z;
use crate::db::Db;
use crate::error::Error;
use crate::product;
use crate::status::check_product_id;

const COLUMNS: &str = "id, title, body, status, kind, product_id, priority, branch, claimed_by, \
                       claim_id, claimed_at, claim_expires_at, commit_sha, verification, \
                       release_tag, created_at, updated_at";

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

/// Create a task in `draft`.
pub fn create(db: &Db, new: &NewTask, now: OffsetDateTime) -> Result<Task, Error> {
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

/// Move a task to `to`, refusing transitions the table forbids and releases the
/// owning product does not want.
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
        if to == TaskStatus::Released {
            let releases = match &task.product_id {
                Some(product_id) => product::read(tx, product_id)?.releases,
                None => false,
            };
            if !releases {
                let product_id = task.product_id.as_deref().unwrap_or("<none>");
                return Err(Error::Invalid(format!(
                    "product {product_id} does not release"
                )));
            }
        }
        tx.execute(
            "UPDATE tasks SET status = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, to.as_str(), stamp],
        )?;
        read(tx, id)
    })
}

/// Hand the next ready task to `worker`, newest lease wins nothing: the row is
/// only taken while it is still `ready`, so no two workers hold the same task.
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
    db.with_tx(|tx| {
        loop {
            let sql = format!(
                "SELECT {COLUMNS} FROM tasks WHERE status = 'ready'
                 ORDER BY CASE kind WHEN 'instant:merge' THEN 0 ELSE 1 END,
                          priority DESC, created_at ASC, id ASC
                 LIMIT 1"
            );
            let Some(task) = query_all(tx, &sql, &[])?.pop() else {
                return Ok(None);
            };
            let claim_id = uuid::Uuid::new_v4().to_string();
            let updated = tx.execute(
                "UPDATE tasks SET status = 'wip', claimed_by = ?2, claim_id = ?3, claimed_at = ?4,
                        claim_expires_at = ?5, updated_at = ?4
                 WHERE id = ?1 AND status = 'ready'",
                rusqlite::params![task.id, worker, claim_id, claimed_at, claim_expires_at],
            )?;
            if updated > 0 {
                return read(tx, &task.id).map(Some);
            }
        }
    })
}

/// Accept a worker's result for the lease `claim_id`.
pub fn report(
    db: &Db,
    claim_id: &str,
    commit_sha: &str,
    verification: &str,
    now: OffsetDateTime,
) -> Result<Task, Error> {
    if commit_sha.trim().is_empty() || verification.trim().is_empty() {
        return Err(Error::Invalid(
            "commit_sha and verification are required".into(),
        ));
    }
    let stamp = format_z(now);
    db.with_tx(|tx| {
        let sql = format!("SELECT {COLUMNS} FROM tasks WHERE claim_id = ?1");
        let Some(task) = query_all(tx, &sql, &[&claim_id])?.pop() else {
            return Err(Error::ClaimMismatch);
        };
        match task.status {
            TaskStatus::Wip => {
                tx.execute(
                    "UPDATE tasks SET status = 'done', commit_sha = ?2, verification = ?3,
                            updated_at = ?4
                     WHERE id = ?1",
                    rusqlite::params![task.id, commit_sha, verification, stamp],
                )?;
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

fn read(conn: &Connection, id: &str) -> Result<Task, Error> {
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
    })
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::{
        NewTask, TaskKind, TaskStatus, can_transition, claim, create, get, list, list_by_status,
        report, set_status,
    };
    use crate::db::Db;
    use crate::error::Error;
    use crate::product::{self, Product};

    fn now() -> time::OffsetDateTime {
        datetime!(2026-03-04 05:06:07 UTC)
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

        let done = report(&db, &claim_id, "abc1234", "cargo test", now()).unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert_eq!(done.commit_sha.as_deref(), Some("abc1234"));

        let again = report(&db, &claim_id, "abc1234", "cargo test", now()).unwrap();
        assert_eq!(again, done);

        assert!(matches!(
            report(&db, &claim_id, "def5678", "cargo test", now()),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            report(&db, "not-a-claim", "abc1234", "cargo test", now()),
            Err(Error::ClaimMismatch)
        ));
        assert!(matches!(
            report(&db, &claim_id, " ", "cargo test", now()),
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
        for to in [
            TaskStatus::Ready,
            TaskStatus::Wip,
            TaskStatus::Done,
            TaskStatus::Merged,
        ] {
            set_status(&db, "t-1", to, now()).unwrap();
        }

        assert!(matches!(
            set_status(&db, "t-1", TaskStatus::Released, now()),
            Err(Error::Invalid(_))
        ));
        assert_eq!(get(&db, "t-1").unwrap().status, TaskStatus::Merged);
    }
}
