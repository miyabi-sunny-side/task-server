use std::fs;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::error::Error;

const SCHEMA_V1: &str = "\
BEGIN;
CREATE TABLE products (
  id           TEXT PRIMARY KEY,
  repository   TEXT NOT NULL,
  description  TEXT NOT NULL DEFAULT '',
  releases     INTEGER NOT NULL DEFAULT 1,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);
CREATE TABLE tasks (
  id                TEXT PRIMARY KEY,
  title             TEXT NOT NULL,
  body              TEXT NOT NULL DEFAULT '',
  status            TEXT NOT NULL,
  kind              TEXT NOT NULL DEFAULT 'normal',
  product_id        TEXT REFERENCES products(id),
  priority          INTEGER NOT NULL DEFAULT 0,
  branch            TEXT,
  claimed_by        TEXT,
  claim_id          TEXT,
  claimed_at        TEXT,
  claim_expires_at  TEXT,
  commit_sha        TEXT,
  verification      TEXT,
  release_tag       TEXT,
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL
);
CREATE INDEX tasks_status_idx ON tasks(status);
CREATE UNIQUE INDEX tasks_claim_id_idx ON tasks(claim_id) WHERE claim_id IS NOT NULL;
PRAGMA user_version = 1;
COMMIT;
";

/// Version 2 adds the merge control plane: a merge task points at the task it
/// lands, and a report carries the checks that justified landing it. The
/// partial unique index is what makes a double issue impossible: at most one
/// live merge may target a task, while cancelled and dropped attempts stay on
/// the record and never block a retry.
const SCHEMA_V2: &str = "\
BEGIN;
ALTER TABLE tasks ADD COLUMN merge_target_task_id TEXT REFERENCES tasks(id);
ALTER TABLE tasks ADD COLUMN checks_json TEXT;
CREATE UNIQUE INDEX tasks_open_merge_target_idx ON tasks(merge_target_task_id)
  WHERE merge_target_task_id IS NOT NULL AND status NOT IN ('cancelled', 'dropped');
PRAGMA user_version = 2;
COMMIT;
";

/// Version 3 rebuilds `tasks` without the `products` foreign key.
///
/// Who writes a task and who curates the catalogue are two different moments:
/// an agent files work for a product that may not be entered yet, and the
/// database has to accept that row. The refusal moves to the `ready` gate, which
/// can explain itself; a foreign key could only answer "constraint failed".
///
/// `merge_target_task_id` keeps its self-referencing key. The new table names
/// `tasks` in that clause while the old `tasks` still exists; after the drop and
/// the rename the clause points at the table itself. Foreign keys are off for
/// the whole rebuild, so sqlite neither enforces the dangling moment nor
/// rewrites the clause during `RENAME`.
///
/// The batch opens the transaction and stops there. Stamping `user_version` and
/// committing belong to [`rebuild_tasks_without_the_product_key`], which does
/// both only after `PRAGMA foreign_key_check` came back empty.
const SCHEMA_V3: &str = "\
BEGIN;
CREATE TABLE tasks_v3 (
  id                   TEXT PRIMARY KEY,
  title                TEXT NOT NULL,
  body                 TEXT NOT NULL DEFAULT '',
  status               TEXT NOT NULL,
  kind                 TEXT NOT NULL DEFAULT 'normal',
  product_id           TEXT,
  priority             INTEGER NOT NULL DEFAULT 0,
  branch               TEXT,
  claimed_by           TEXT,
  claim_id             TEXT,
  claimed_at           TEXT,
  claim_expires_at     TEXT,
  commit_sha           TEXT,
  verification         TEXT,
  release_tag          TEXT,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL,
  merge_target_task_id TEXT REFERENCES tasks(id),
  checks_json          TEXT
);
INSERT INTO tasks_v3 (id, title, body, status, kind, product_id, priority, branch, claimed_by,
                      claim_id, claimed_at, claim_expires_at, commit_sha, verification,
                      release_tag, created_at, updated_at, merge_target_task_id, checks_json)
  SELECT id, title, body, status, kind, product_id, priority, branch, claimed_by,
         claim_id, claimed_at, claim_expires_at, commit_sha, verification,
         release_tag, created_at, updated_at, merge_target_task_id, checks_json
  FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_v3 RENAME TO tasks;
CREATE INDEX tasks_status_idx ON tasks(status);
CREATE UNIQUE INDEX tasks_claim_id_idx ON tasks(claim_id) WHERE claim_id IS NOT NULL;
CREATE UNIQUE INDEX tasks_open_merge_target_idx ON tasks(merge_target_task_id)
  WHERE merge_target_task_id IS NOT NULL AND status NOT IN ('cancelled', 'dropped');
";

/// Version 4 adds the archive column to `products`.
///
/// A product whose working copy left the disk may not be deleted: tasks that
/// already named it — merged, released, part of the record — would be left
/// pointing at nothing, and `product_id` carries no foreign key that would have
/// stopped it. So the row stays and is marked instead. `NULL` is live; a
/// timestamp is the moment the walk stopped finding it. Every row that predates
/// the column is live, which is what `ALTER TABLE` fills in.
const SCHEMA_V4: &str = "\
BEGIN;
ALTER TABLE products ADD COLUMN archived_at TEXT;
PRAGMA user_version = 4;
COMMIT;
";

/// Version 5 adds the review control plane: a review task points at the work it
/// reads and records the verdict it answered with.
///
/// The review keeps a target column of its own rather than sharing the merge's,
/// because the two are live for different spans. A merge owns its target once it
/// has landed, so `done` still blocks a second one; a review is over the moment
/// it answers, and a task sent back for changes has to be reviewable again. One
/// column could not carry both predicates, and a finished review sharing the
/// merge column would have blocked the merge that follows it.
///
/// The subject of a review — the commit it was issued for — needs no column: it
/// is the review's own `commit_sha`, inherited from the target exactly as a
/// merge inherits one, and the findings are the review's `verification`, which
/// is already the column for evidence a human reads.
///
/// `review_attempt` is the one thing the id cannot say. Retries are derived ids
/// — `review:t-1`, `review:t-1~2`, `review:t-1~3` — and comparing those as text
/// puts `~9` after `~10`, so "the latest review" would go backwards on the tenth
/// attempt. The attempt number is written as an integer instead, and it is the
/// order `latest_review` reads by. Timestamps could not stand in for it: they
/// are second-precision, so two attempts finished in the same second are a tie.
const SCHEMA_V5: &str = "\
BEGIN;
ALTER TABLE tasks ADD COLUMN review_target_task_id TEXT REFERENCES tasks(id);
ALTER TABLE tasks ADD COLUMN review_verdict TEXT;
ALTER TABLE tasks ADD COLUMN review_attempt INTEGER;
CREATE UNIQUE INDEX tasks_open_review_target_idx ON tasks(review_target_task_id)
  WHERE review_target_task_id IS NOT NULL
    AND status NOT IN ('done', 'cancelled', 'dropped');
PRAGMA user_version = 5;
COMMIT;
";

/// Version 6 adds the merge train: the order merges were issued in, per product.
///
/// A merge rebases its branch onto the main line, so two merges of one product
/// cannot run at once and cannot run out of order — the second would be rebasing
/// onto a main line the first has not written yet. The queue therefore hands out
/// one merge per product at a time, in issue order, and that order needs a
/// column of its own.
///
/// Neither of the two orders already on the row can carry it. `created_at` is
/// written to the second, so two merges issued in the same second tie; the id is
/// derived from the target's name, so it sorts alphabetically and puts `~10`
/// before `~9`. `merge_sequence` is a single counter over the whole table rather
/// than one per product: what a train needs is a strict order among *its own*
/// merges, and a global counter gives every product that for free.
///
/// The backfill numbers the merges a database already holds by `created_at` and
/// then by id — the only evidence history left — so an upgraded database comes
/// out with a complete, distinct order instead of a column of `NULL`s that no
/// comparison could rank. `NULL` survives only on rows that are not merges.
const SCHEMA_V6: &str = "\
BEGIN;
ALTER TABLE tasks ADD COLUMN merge_sequence INTEGER;
UPDATE tasks SET merge_sequence = (
  SELECT count(*) FROM tasks AS earlier
  WHERE earlier.kind = 'instant:merge'
    AND (earlier.created_at < tasks.created_at
         OR (earlier.created_at = tasks.created_at AND earlier.id <= tasks.id))
) WHERE kind = 'instant:merge';
PRAGMA user_version = 6;
COMMIT;
";

/// Version 7 drops `merge_sequence`.
///
/// The column held a strict issue order for each product's merges, and nothing
/// needs that order any more: a merge waits only while another merge of the same
/// product is actually running or jammed, and which of several `ready` merges
/// goes first is decided by whoever claims one. What the column stored was never
/// independent evidence either — [`SCHEMA_V6`] derived it from `created_at` and
/// the id, both of which are still on the row.
///
/// Dropping it rather than leaving it unread is the point: a column nobody
/// writes is a question every later reader has to answer.
const SCHEMA_V7: &str = "\
BEGIN;
ALTER TABLE tasks DROP COLUMN merge_sequence;
PRAGMA user_version = 7;
COMMIT;
";

/// Version 8 remembers successful idempotent claim requests.
///
/// A worker may lose the HTTP response after the task has already moved to
/// `wip`. The receipt lets a retry recover that same lease instead of consuming
/// another task. Empty-queue answers are not stored: they changed no state and
/// workers poll often, so recording them would grow this table without a lease
/// to recover.
const SCHEMA_V8: &str = "\
BEGIN;
CREATE TABLE claim_receipts (
  idempotency_key TEXT PRIMARY KEY,
  worker          TEXT NOT NULL,
  kinds           TEXT NOT NULL,
  task_id         TEXT NOT NULL REFERENCES tasks(id),
  claim_id        TEXT NOT NULL UNIQUE,
  created_at      TEXT NOT NULL
);
PRAGMA user_version = 8;
COMMIT;
";

/// Version 9 sweeps the husks a database already has: a `review` or
/// `instant:merge` subtask left at `done` whose target already shipped, from
/// before [`task::release_product`](crate::task::release_product) carried
/// subtasks along with their target. The verdict already lives on the
/// target's `latest_review` and the merge already landed, so a `done` husk
/// says nothing the target's own row does not; this backfill leaves it
/// `released` with the target's own tag and timestamp instead, so a database
/// opened after the fix reads the same as one that never had the bug.
const SCHEMA_V9: &str = "\
BEGIN;
UPDATE tasks
SET status = 'released', release_tag = target.release_tag, updated_at = target.updated_at
FROM tasks AS target
WHERE tasks.kind IN ('review', 'instant:merge') AND tasks.status = 'done'
  AND target.kind = 'normal' AND target.status = 'released'
  AND target.id = COALESCE(tasks.review_target_task_id, tasks.merge_target_task_id);
PRAGMA user_version = 9;
COMMIT;
";

/// Version 10 puts the release on the task. `release_level` is how far shipping
/// the work steps the version, known when the work is filed; `release_task_id`
/// points a landed task at the release that ships it. Landing now ends work of
/// a product that does not release, so the backfill takes the `merged` work of
/// those products — and the finished subtasks under it — to `released` with no
/// tag, exactly where a landing after this version leaves it.
const SCHEMA_V10: &str = "\
BEGIN;
ALTER TABLE tasks ADD COLUMN release_level TEXT NOT NULL DEFAULT 'patch';
ALTER TABLE tasks ADD COLUMN release_task_id TEXT REFERENCES tasks(id);
UPDATE tasks SET status = 'released'
WHERE kind = 'normal' AND status = 'merged'
  AND product_id IN (SELECT id FROM products WHERE releases = 0);
UPDATE tasks SET status = 'released', updated_at = target.updated_at
FROM tasks AS target
WHERE tasks.kind IN ('review', 'instant:merge') AND tasks.status = 'done'
  AND target.kind = 'normal' AND target.status = 'released' AND target.release_tag IS NULL
  AND target.id = COALESCE(tasks.review_target_task_id, tasks.merge_target_task_id);
PRAGMA user_version = 10;
COMMIT;
";

/// Version 11 lets a task wait for another: `depends_on` names the task whose
/// landing promotes this one to `ready`. A column only — nothing a database
/// already holds waits for anything.
const SCHEMA_V11: &str = "\
BEGIN;
ALTER TABLE tasks ADD COLUMN depends_on TEXT REFERENCES tasks(id);
PRAGMA user_version = 11;
COMMIT;
";

/// The one-open-review index as every version from 12 on spells it: `released`
/// is over, like `done`, `cancelled` and `dropped`. The version 5 index stopped
/// at `done`, so a target reviewed twice (`request_changes`, then approve) could
/// not be shipped — moving both finished reviews to `released` collided on the
/// index and the whole release report rolled back while the tag was already on
/// origin. Rewriting an index is idempotent, so this is also run — in its own
/// transaction — before the version 9 and 10 backfills, which move `done`
/// reviews to `released` and would hit the same collision on any database that
/// already carries the index (version 5 to 9) when it upgrades through them.
const REVIEW_INDEX_RELEASED_IS_OVER: &str = "\
BEGIN;
DROP INDEX IF EXISTS tasks_open_review_target_idx;
CREATE UNIQUE INDEX tasks_open_review_target_idx ON tasks(review_target_task_id)
  WHERE review_target_task_id IS NOT NULL
    AND status NOT IN ('done', 'cancelled', 'dropped', 'released');
COMMIT;
";

/// Version 12 changes nothing but the one-open-review index: see
/// [`REVIEW_INDEX_RELEASED_IS_OVER`]. No row moves.
const SCHEMA_V12: &str = "\
BEGIN;
DROP INDEX IF EXISTS tasks_open_review_target_idx;
CREATE UNIQUE INDEX tasks_open_review_target_idx ON tasks(review_target_task_id)
  WHERE review_target_task_id IS NOT NULL
    AND status NOT IN ('done', 'cancelled', 'dropped', 'released');
PRAGMA user_version = 12;
COMMIT;
";

/// Version 13 remembers when a `normal` task first reached `done`.
///
/// `updated_at` cannot stand in for this: it moves again on every later
/// transition (approval, landing, release), so it answers "when did this row
/// last change" rather than "when did the work finish". A row already sitting
/// at `done` or past it earned that status through `done` at some point, so
/// its `updated_at` at migration time is the closest recoverable estimate —
/// history that predates this column was never asked to keep a better one. A
/// row that never reached `done` gets no estimate at all: guessing one for
/// `draft`, `ready`, `wip`, or a sidetracked row that was never `done` would
/// assert a completion that did not happen.
const SCHEMA_V13: &str = "\
BEGIN;
ALTER TABLE tasks ADD COLUMN done_at TEXT;
UPDATE tasks SET done_at = updated_at
  WHERE kind = 'normal' AND status IN ('done', 'approved', 'merged', 'released');
PRAGMA user_version = 13;
COMMIT;
";

/// How long a writer waits for a lock before giving up, in milliseconds.
const BUSY_TIMEOUT_MS: i64 = 5000;

/// The one path that asks for an in-memory database.
///
/// Nothing else spells it. In particular the URI forms sqlite also understands
/// — `file::memory:`, `file:name?mode=memory` — are **not** recognised here as
/// in-memory: they are classified as files, so the WAL read-back refuses them
/// rather than letting a database with no file behind it through.
const IN_MEMORY_PATH: &str = ":memory:";

/// Whether a database has a file behind it.
///
/// Continuous backup follows the write-ahead log, so a database on disk that is
/// not in WAL is refused outright: a replica of a `delete` mode database would
/// silently stop tracking the truth. An in-memory database has no file to
/// replicate and keeps whatever journal mode sqlite gives it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Backing {
    File,
    Memory,
}

/// The sqlite database that holds the control plane state.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// Open (creating if needed) the database at `path`, running migrations.
    ///
    /// # Errors
    ///
    /// Fails when `path` is blank, when the file cannot be opened, when it
    /// cannot be put into WAL mode, or when a migration is refused.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let backing = backing_asked_for(path)?;
        if backing == Backing::File
            && let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        Self::from_connection(Connection::open(path)?, backing)
    }

    /// Open a private in-memory database, running migrations.
    ///
    /// # Errors
    ///
    /// Fails when a migration is refused.
    pub fn open_in_memory() -> Result<Self, Error> {
        Self::from_connection(Connection::open_in_memory()?, Backing::Memory)
    }

    fn from_connection(conn: Connection, backing: Backing) -> Result<Self, Error> {
        conn.execute_batch(&format!(
            "PRAGMA foreign_keys=ON; PRAGMA busy_timeout={BUSY_TIMEOUT_MS};"
        ))?;
        if backing == Backing::File {
            // `PRAGMA journal_mode` answers with the mode it settled on, so it
            // is a query, not a statement to execute and forget.
            conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))
                .map_err(|error| {
                    Error::Db(format!("the database could not enter WAL mode: {error}"))
                })?;
        }
        confirm_pragmas(&conn, backing)?;
        migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let conn = self.conn.lock().expect("db mutex");
        f(&conn)
    }

    /// Run `f` inside an immediate transaction, so concurrent writers serialize
    /// at `BEGIN` instead of failing to upgrade a read lock later.
    pub(crate) fn with_tx<T>(
        &self,
        f: impl FnOnce(&Transaction) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut conn = self.conn.lock().expect("db mutex");
        let tx = Transaction::new(&mut conn, TransactionBehavior::Immediate)?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    }
}

/// Decide from the requested path what kind of database the caller asked for.
///
/// The decision cannot be left to sqlite after the fact: `Connection::path`
/// answers `Some("")` for a *temporary* database just as it does for an
/// in-memory one, so reading the exemption off that report made `Db::open("")`
/// succeed against a scratch database that is neither replicated nor persisted,
/// with the WAL requirement skipped. Only [`IN_MEMORY_PATH`] asks for memory;
/// every other path is a file and must be in WAL. A blank path asks for
/// nothing at all and is refused here, before a connection exists.
fn backing_asked_for(path: &Path) -> Result<Backing, Error> {
    match path.to_str() {
        Some(text) if text.trim().is_empty() => Err(Error::Invalid(format!(
            "a database path is required: name a file, or `{IN_MEMORY_PATH}` for an in-memory database"
        ))),
        Some(text) if text == IN_MEMORY_PATH => Ok(Backing::Memory),
        _ => Ok(Backing::File),
    }
}

/// Read back what the open asked for, because a pragma that was refused, or
/// quietly ignored, is indistinguishable from one that was applied.
///
/// The write-ahead log itself is left alone: no `wal_autocheckpoint` is set and
/// no checkpoint is ever forced. Trimming the log is sqlite's business, and a
/// deployment that replicates the log needs the replicator to have read a frame
/// before it goes.
fn confirm_pragmas(conn: &Connection, backing: Backing) -> Result<(), Error> {
    let busy_timeout: i64 = conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
    if busy_timeout != BUSY_TIMEOUT_MS {
        return Err(Error::Db(format!(
            "busy_timeout is {busy_timeout}ms, not the {BUSY_TIMEOUT_MS}ms writers are serialized on"
        )));
    }
    let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if backing == Backing::File && !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(Error::Db(format!(
            "a database on disk must be in WAL mode so a backup can follow it, but this one is in {journal_mode} mode"
        )));
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<(), Error> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 1 {
        conn.execute_batch(SCHEMA_V1)?;
    }
    if version < 2 {
        conn.execute_batch(SCHEMA_V2)?;
    }
    if version < 3 {
        rebuild_tasks_without_the_product_key(conn)?;
    }
    if version < 4 {
        conn.execute_batch(SCHEMA_V4)?;
    }
    if version < 5 {
        conn.execute_batch(SCHEMA_V5)?;
    }
    if version < 6 {
        conn.execute_batch(SCHEMA_V6)?;
    }
    if version < 7 {
        conn.execute_batch(SCHEMA_V7)?;
    }
    if version < 8 {
        conn.execute_batch(SCHEMA_V8)?;
    }
    if (5..10).contains(&version) {
        // The version 9 and 10 backfills move `done` reviews to `released`; on
        // the version 5 index two of them for one target would collide. Widen
        // the index first on every database that already has it (a database
        // below 5 has no review rows yet, so its backfills move nothing) —
        // version 12 does the same again, harmlessly.
        conn.execute_batch(REVIEW_INDEX_RELEASED_IS_OVER)?;
    }
    if version < 9 {
        conn.execute_batch(SCHEMA_V9)?;
    }
    if version < 10 {
        conn.execute_batch(SCHEMA_V10)?;
    }
    if version < 11 {
        conn.execute_batch(SCHEMA_V11)?;
    }
    if version < 12 {
        conn.execute_batch(SCHEMA_V12)?;
    }
    if version < 13 {
        conn.execute_batch(SCHEMA_V13)?;
    }
    Ok(())
}

/// Run [`SCHEMA_V3`] under sqlite's table rebuild procedure.
///
/// `PRAGMA foreign_keys` is a no-op inside a transaction, so it is set around
/// the batch, not within it. Everything else stays inside one transaction:
/// `foreign_key_check` has to come back empty *before* the version is stamped
/// and the work committed. A rebuild that stranded a merge target is a failed
/// migration, so it rolls back whole — the version stays where it was and the
/// next open runs the same check instead of skipping it on a version the
/// database never earned.
fn rebuild_tasks_without_the_product_key(conn: &Connection) -> Result<(), Error> {
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
    let rebuilt = rebuild_then_stamp(conn);
    if rebuilt.is_err() {
        // Either the batch died with its transaction still open, or the check
        // refused the rebuild we opened it for.
        drop(conn.execute_batch("ROLLBACK;"));
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    rebuilt
}

/// The transactional half: rebuild, verify, then stamp and commit. A `PRAGMA
/// user_version` write lives in the database header and rolls back with the
/// transaction, so a failure here leaves version 2 behind.
fn rebuild_then_stamp(conn: &Connection) -> Result<(), Error> {
    conn.execute_batch(SCHEMA_V3)?;
    check_references(conn)?;
    conn.execute_batch("PRAGMA user_version = 3; COMMIT;")?;
    Ok(())
}

fn check_references(conn: &Connection) -> Result<(), Error> {
    let mut statement = conn.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    if let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        return Err(Error::Db(format!(
            "migration to version 3 left a dangling reference in {table}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use time::macros::datetime;

    use super::{
        Db, SCHEMA_V1, SCHEMA_V2, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8, SCHEMA_V9,
        SCHEMA_V10, SCHEMA_V11,
    };

    fn pragma_string(db: &Db, pragma: &str) -> String {
        db.with_conn(|conn| Ok(conn.query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))?))
            .unwrap()
    }

    fn pragma_int(db: &Db, pragma: &str) -> i64 {
        db.with_conn(|conn| Ok(conn.query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))?))
            .unwrap()
    }

    fn user_version(db: &Db) -> i64 {
        db.with_conn(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?))
            .unwrap()
    }

    /// Continuous backup reads the WAL, so a database on disk that is not in WAL
    /// is not a database this server may serve: the replica would silently fall
    /// behind the truth. `Db::open` therefore reads the mode back and refuses
    /// rather than carrying on in `delete` mode. An in-memory database has no
    /// WAL to keep and is exempt.
    #[test]
    fn a_file_backed_database_is_wal_or_it_does_not_open() {
        let dir = tempfile::tempdir().unwrap();

        let db = Db::open(dir.path().join("sqlite.db")).unwrap();
        assert_eq!(pragma_string(&db, "journal_mode"), "wal");
        assert_eq!(pragma_int(&db, "busy_timeout"), 5000);
        drop(db);

        let memory = Db::open_in_memory().unwrap();
        assert_eq!(
            pragma_string(&memory, "journal_mode"),
            "memory",
            "an in-memory database keeps its own journal mode"
        );
        assert_eq!(pragma_int(&memory, "busy_timeout"), 5000);

        // `APP_DB_PATH=:memory:` arrives through the same door as a file and is
        // still not a file, so it is exempt too.
        let named_memory = Db::open(":memory:").unwrap();
        assert_eq!(pragma_string(&named_memory, "journal_mode"), "memory");

        // Sqlite refuses to change journal mode while another connection holds
        // the file, so a `delete` database with a reader on it is a disk
        // database that cannot reach WAL. Reading it still works — only a
        // deliberate check of the mode catches this.
        let contested = dir.path().join("contested.db");
        drop(Db::open(&contested).unwrap());
        let reader = Connection::open(&contested).unwrap();
        let mode: String = reader
            .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "delete", "the fixture must start outside WAL");
        reader
            .execute_batch("BEGIN; SELECT count(*) FROM products;")
            .unwrap();

        let Err(error) = Db::open(&contested) else {
            panic!("a disk database that cannot be WAL must not open");
        };
        assert!(
            matches!(error, crate::error::Error::Db(_)),
            "a database that cannot be WAL must be refused as a database fault, got {error:?}"
        );
    }

    /// Sqlite reports a temporary database exactly like an in-memory one: no
    /// filename at all. Reading the exemption off that report let an empty
    /// `APP_DB_PATH` open a temporary database in `delete` mode — neither
    /// replicated nor persisted — with the WAL requirement quietly skipped. A
    /// path is required, and only an explicit in-memory database is exempt.
    #[test]
    fn a_blank_path_is_refused_and_only_an_explicit_memory_database_is_exempt() {
        for blank in ["", " ", "\t\n"] {
            let Err(error) = Db::open(blank) else {
                panic!(
                    "a blank database path must be refused, not opened as a temporary database ({blank:?})"
                );
            };
            assert!(
                matches!(error, crate::error::Error::Invalid(_)),
                "a blank path is a bad argument, not a database fault, got {error:?}"
            );
        }

        // The explicit spellings stay exempt, and still migrate.
        let named = Db::open(":memory:").unwrap();
        assert_eq!(pragma_string(&named, "journal_mode"), "memory");
        assert_eq!(pragma_int(&named, "busy_timeout"), 5000);
        assert_eq!(user_version(&named), 13);

        let private = Db::open_in_memory().unwrap();
        assert_eq!(pragma_int(&private, "busy_timeout"), 5000);
        assert_eq!(user_version(&private), 13);

        // A URI spelling is not one of them. Only the exact `:memory:` is
        // exempt, so a URI is an ordinary filename: it lands on disk in WAL,
        // where a replica can follow it, rather than becoming an unbacked
        // database that quietly holds the control plane in RAM.
        let dir = tempfile::tempdir().unwrap();
        for uri in ["file::memory:", "file:cache?mode=memory"] {
            let asked = dir.path().join(uri);
            let db = Db::open(&asked).expect("a URI spelling is just a filename");
            assert_eq!(
                pragma_string(&db, "journal_mode"),
                "wal",
                "a URI memory spelling must not become an in-memory database ({uri})"
            );
            assert!(asked.is_file(), "{uri} must have produced a real file");
        }
    }

    /// Every column of one task row, in schema order, as `(name, value)`. A
    /// rebuild that dropped, reordered, or blanked a column shows up here.
    /// The columns versions 10, 11 and 13 added. Every migration test below
    /// asks about an earlier step, and a column that arrived after it would
    /// otherwise show up as a difference in every row comparison.
    const LATER_COLUMNS: [&str; 4] = ["release_level", "release_task_id", "depends_on", "done_at"];

    fn task_row(conn: &Connection, id: &str) -> Vec<(String, String)> {
        let mut statement = conn.prepare("SELECT * FROM tasks WHERE id = ?1").unwrap();
        let names: Vec<String> = statement
            .column_names()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        let mut rows = statement.query([id]).unwrap();
        let row = rows.next().unwrap().expect("task row");
        names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let value: rusqlite::types::Value = row.get(index).unwrap();
                (name.clone(), format!("{value:?}"))
            })
            .filter(|(name, _)| !LATER_COLUMNS.contains(&name.as_str()))
            .collect()
    }

    /// One task row without the columns a version later than the one under test
    /// added. A migration test asks whether the step it names carried the row
    /// across, and every step after it would otherwise show up as a difference.
    fn row_before_later_versions(conn: &Connection, id: &str) -> Vec<(String, String)> {
        task_row(conn, id)
            .into_iter()
            .filter(|(name, _)| !name.starts_with("review_"))
            .collect()
    }

    /// The rows of several tasks, without the columns a later version added:
    /// the version 3 rebuild is what is under test, not what came after it.
    fn rebuilt_rows(conn: &Connection, ids: [&str; 3]) -> Vec<Vec<(String, String)>> {
        ids.into_iter()
            .map(|id| row_before_later_versions(conn, id))
            .collect()
    }

    /// The columns a table has, as the database reports them. Read from the
    /// schema rather than inferred, so a migration can be asked whether it ran.
    fn column_names(conn: &Connection, table: &str) -> Vec<String> {
        let mut statement = conn
            .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
            .unwrap();
        let rows = statement.query_map([table], |row| row.get(0)).unwrap();
        rows.collect::<Result<Vec<String>, _>>().unwrap()
    }

    fn index_names(conn: &Connection) -> Vec<String> {
        let mut statement = conn
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'index' AND tbl_name = 'tasks' AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .unwrap();
        let rows = statement.query_map([], |row| row.get(0)).unwrap();
        rows.collect::<Result<Vec<String>, _>>().unwrap()
    }

    #[test]
    fn migration_creates_the_current_schema() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(user_version(&db), 13);
        let tables: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM sqlite_master
                     WHERE type = 'table' AND name IN ('products', 'tasks', 'claim_receipts')",
                    [],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(tables, 3);
    }

    #[test]
    fn migration_is_idempotent_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/sqlite.db");

        let db = Db::open(&path).unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO products (id, repository, description, releases, created_at, updated_at)
                 VALUES ('a/b', 'https://example.test/a/b.git', '', 1, 'now', 'now')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        drop(db);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        let products: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT count(*) FROM products", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(products, 1);
    }

    /// A database written by the first release must move to the current version
    /// with every row intact, so an upgrade is never a data migration by hand.
    #[test]
    fn a_version_one_database_upgrades_without_losing_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(SCHEMA_V1).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO products (id, repository, description, releases, created_at, updated_at)
                 VALUES ('a/b', 'https://example.test/a/b.git', 'kept', 1, 'then', 'then');
                 INSERT INTO tasks (id, title, body, status, kind, product_id, priority, branch,
                                    commit_sha, verification, created_at, updated_at)
                 VALUES ('t-old', 'older than the migration', 'body', 'done', 'normal', 'a/b', 4,
                         'task/t-old', 'abc1234', 'cargo test', 'then', 'then');",
            )
            .unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 1, "the fixture must start at version 1");
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);

        let (title, status, priority, merge_target, checks): (
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
        ) = db
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT title, status, priority, merge_target_task_id, checks_json
                     FROM tasks WHERE id = 't-old'",
                    [],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )?)
            })
            .unwrap();
        assert_eq!(title, "older than the migration");
        assert_eq!(status, "done");
        assert_eq!(priority, 4);
        assert!(merge_target.is_none(), "the new column starts empty");
        assert!(checks.is_none(), "the new column starts empty");

        let products: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT count(*) FROM products", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(products, 1);

        let index: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM sqlite_master
                     WHERE type = 'index' AND name = 'tasks_open_merge_target_idx'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(index, 1, "version 2 owns the single-live-merge index");
    }

    /// Version 3 drops only the `products` foreign key from `tasks`, because a
    /// task may be registered for a product the catalogue has never heard of.
    /// The rebuild has to carry every column across, keep the indexes, and leave
    /// the self-referencing merge target key enforced.
    #[test]
    fn migrating_to_v3_keeps_every_row_and_accepts_an_uncatalogued_product() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(SCHEMA_V1).unwrap();
        legacy.execute_batch(SCHEMA_V2).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO products (id, repository, description, releases, created_at, updated_at)
                 VALUES ('a/b', 'https://example.test/a/b.git', 'kept', 1, 'then', 'then');
                 INSERT INTO tasks (id, title, body, status, kind, product_id, priority, branch,
                                    claimed_by, claim_id, claimed_at, claim_expires_at, commit_sha,
                                    verification, release_tag, created_at, updated_at,
                                    merge_target_task_id, checks_json)
                 VALUES ('t-target', 'landed work', '本文', 'merged', 'normal', 'a/b', 7,
                         'task/t-target', 'grok', 'claim-1', 'then', 'later', 'abc1234',
                         'cargo test', 'v0.1.0', 'then', 'later', NULL, NULL);
                 INSERT INTO tasks (id, title, body, status, kind, product_id, priority, branch,
                                    claimed_by, claim_id, claimed_at, claim_expires_at, commit_sha,
                                    verification, release_tag, created_at, updated_at,
                                    merge_target_task_id, checks_json)
                 VALUES ('merge:t-target', 'merge t-target', '', 'done', 'instant:merge', 'a/b', 7,
                         'task/t-target', 'grok', 'claim-2', 'then', 'later', 'abc1234',
                         'cargo test', NULL, 'then', 'later', 't-target',
                         '[{\"name\":\"cargo test\",\"exit_code\":0}]');
                 INSERT INTO tasks (id, title, status, created_at, updated_at)
                 VALUES ('t-plain', 'no product at all', 'draft', 'then', 'then');",
            )
            .unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 2, "the fixture must start at version 2");
        let rows_before = rebuilt_rows(&legacy, ["t-target", "merge:t-target", "t-plain"]);
        let indexes_before = index_names(&legacy);
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);

        db.with_conn(|conn| {
            let rows_after = rebuilt_rows(conn, ["t-target", "merge:t-target", "t-plain"]);
            assert_eq!(
                rows_after, rows_before,
                "the rebuild must keep every column"
            );
            assert_eq!(
                indexes_before,
                [
                    "tasks_claim_id_idx",
                    "tasks_open_merge_target_idx",
                    "tasks_status_idx"
                ],
                "the fixture must start with all three indexes"
            );
            // Exactly the three the rebuild had to recreate plus the one
            // version 5 adds — no index quietly dropped, none quietly gained.
            let mut expected = indexes_before.clone();
            expected.push("tasks_open_review_target_idx".to_owned());
            expected.sort();
            assert_eq!(
                index_names(conn),
                expected,
                "the migrated database must carry the old indexes and the review index, and \
                 nothing else"
            );

            let products: i64 =
                conn.query_row("SELECT count(*) FROM products", [], |row| row.get(0))?;
            assert_eq!(products, 1, "products are untouched by the rebuild");

            let mut check = conn.prepare("PRAGMA foreign_key_check")?;
            let dangling: i64 = check.query([])?.next()?.map_or(0, |_| 1);
            assert_eq!(dangling, 0, "the migrated database must reference cleanly");
            Ok(())
        })
        .unwrap();

        // The point of version 3: a task may name a product the catalogue does
        // not carry yet, because the refusal belongs to the `ready` gate.
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, product_id, created_at, updated_at)
                 VALUES ('t-unlisted', 'unlisted product', 'draft', 'nobody/knows', 'now', 'now')",
                [],
            )?;
            Ok(())
        })
        .expect("an uncatalogued product_id must be accepted");

        // The merge target key is still a foreign key, and still enforced.
        let dangling_merge = db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, merge_target_task_id, created_at, updated_at)
                 VALUES ('merge:ghost', 'merge a ghost', 'ready', 't-missing', 'now', 'now')",
                [],
            )?;
            Ok(())
        });
        assert!(
            dangling_merge.is_err(),
            "the self-referencing merge target key must stay enforced"
        );
    }

    /// A product whose working copy is gone is archived, not deleted: the tasks
    /// that named it still have to resolve. Version 4 is the column that holds
    /// that, and every row a database already has is live — nothing was archived
    /// before the column existed.
    ///
    /// The fixture is a real version 3 database — stamped 3 before the open, with
    /// no `archived_at` on `products` — because that is the migration under test.
    /// A version 2 fixture would reach version 4 through a different path and
    /// prove nothing about the step from 3.
    #[test]
    fn a_version_three_database_gains_an_empty_archive_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(SCHEMA_V1).unwrap();
        legacy.execute_batch(SCHEMA_V2).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO products (id, repository, description, releases, created_at, updated_at)
                 VALUES ('a/b', 'https://example.test/a/b.git', 'kept', 1, 'then', 'then');
                 INSERT INTO tasks (id, title, status, product_id, created_at, updated_at)
                 VALUES ('t-1', 'landed work', 'merged', 'a/b', 'then', 'then');",
            )
            .unwrap();
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 3, "the fixture must start at version 3");
        assert!(
            !column_names(&legacy, "products").contains(&"archived_at".to_owned()),
            "a version 3 database has no archive column yet"
        );
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        let archived = |db: &Db| -> (usize, Option<String>) {
            db.with_conn(|conn| {
                let columns = column_names(conn, "products");
                Ok((
                    columns.iter().filter(|name| *name == "archived_at").count(),
                    conn.query_row(
                        "SELECT archived_at FROM products WHERE id = 'a/b'",
                        [],
                        |row| row.get(0),
                    )?,
                ))
            })
            .unwrap()
        };
        assert_eq!(
            archived(&db),
            (1, None),
            "the column is added once, and a row that predates it is live"
        );

        // Reopening runs the migration step again, and must be a no-op: the
        // column is not added twice and the row is not re-marked.
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        assert_eq!(archived(&db), (1, None), "a second open changes nothing");
        let tasks: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM tasks t JOIN products p ON p.id = t.product_id",
                    [],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(
            tasks, 1,
            "the history the column exists to keep still joins"
        );
    }

    /// A rebuild that would strand a merge target is a failed migration, and a
    /// failed migration has to leave nothing behind: the version is not stamped,
    /// the rows are untouched, and the next open runs the same check again
    /// instead of skipping it on a version the database never earned.
    #[test]
    fn a_dangling_merge_target_fails_the_migration_and_leaves_version_two() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(SCHEMA_V1).unwrap();
        legacy.execute_batch(SCHEMA_V2).unwrap();
        // Foreign keys off is what lets the fixture record the damage a real
        // database can carry: a merge pointing at a task that is not there.
        legacy.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        legacy
            .execute_batch(
                "INSERT INTO tasks (id, title, status, kind, merge_target_task_id,
                                    created_at, updated_at)
                 VALUES ('merge:ghost', 'merge a ghost', 'ready', 'instant:merge', 't-missing',
                         'then', 'then');",
            )
            .unwrap();
        drop(legacy);

        assert!(
            Db::open(&path).is_err(),
            "a dangling merge target must fail the migration"
        );
        assert!(
            Db::open(&path).is_err(),
            "the second open must fail the same way, not inherit a stamped version"
        );

        let after = Connection::open(&path).unwrap();
        let version: i64 = after
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2, "a refused migration must not stamp its version");
        let target: Option<String> = after
            .query_row(
                "SELECT merge_target_task_id FROM tasks WHERE id = 'merge:ghost'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            target.as_deref(),
            Some("t-missing"),
            "the row must survive the rollback exactly as it was"
        );
    }

    /// The index has to refuse a second live merge for one target while still
    /// allowing a retry after the first attempt was abandoned.
    #[test]
    fn at_most_one_live_merge_targets_a_task() {
        let db = Db::open_in_memory().unwrap();
        let insert = |id: &str, status: &str| {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO tasks (id, title, status, kind, merge_target_task_id,
                                        created_at, updated_at)
                     VALUES (?1, 'merge', ?2, 'instant:merge', 't-target', 'now', 'now')",
                    rusqlite::params![id, status],
                )?;
                Ok(())
            })
        };
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, created_at, updated_at)
                 VALUES ('t-target', 'target', 'done', 'now', 'now')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        insert("m-1", "ready").unwrap();
        assert!(
            insert("m-2", "ready").is_err(),
            "a second live merge for the same target must be refused"
        );

        db.with_conn(|conn| {
            conn.execute("UPDATE tasks SET status = 'dropped' WHERE id = 'm-1'", [])?;
            Ok(())
        })
        .unwrap();
        insert("m-2", "ready").expect("a dropped attempt must not block a retry");
    }

    /// Foreign keys are on, and the merge target key is the one `tasks` keeps.
    /// `product_id` is deliberately not one: a task may be registered against a
    /// product the catalogue does not carry, and the `ready` gate is what
    /// refuses it later.
    #[test]
    fn foreign_keys_are_enforced_except_on_product_id() {
        let db = Db::open_in_memory().unwrap();
        let enforced: bool = db
            .with_conn(|conn| Ok(conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?))
            .unwrap();
        assert!(enforced, "foreign keys must be enforced");

        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, created_at, updated_at, product_id)
                 VALUES ('t-1', 'title', 'draft', 'now', 'now', 'missing/product')",
                [],
            )?;
            Ok(())
        })
        .expect("an uncatalogued product_id is registered, not refused");

        let dangling_merge = db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, merge_target_task_id, created_at, updated_at)
                 VALUES ('merge:ghost', 'merge a ghost', 'ready', 't-missing', 'now', 'now')",
                [],
            )?;
            Ok(())
        });
        assert!(
            dangling_merge.is_err(),
            "a merge pointing at no task must be rejected"
        );
    }

    #[test]
    fn with_tx_rolls_back_on_error() {
        let db = Db::open_in_memory().unwrap();
        let result: Result<(), _> = db.with_tx(|tx| {
            tx.execute(
                "INSERT INTO products (id, repository, description, releases, created_at, updated_at)
                 VALUES ('a/b', 'https://example.test/a/b.git', '', 1, 'now', 'now')",
                [],
            )?;
            Err(crate::error::Error::Invalid("nope".into()))
        });
        assert!(result.is_err());
        let products: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT count(*) FROM products", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(products, 0);
    }

    /// Version 5 adds the review control plane. Everything a version 4 database
    /// already holds has to survive it, and the two new columns start empty:
    /// nothing was reviewed before they existed.
    #[test]
    fn a_version_four_database_gains_empty_review_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(SCHEMA_V1).unwrap();
        legacy.execute_batch(SCHEMA_V2).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO products (id, repository, description, releases, created_at, updated_at)
                 VALUES ('a/b', 'https://example.test/a/b.git', 'kept', 1, 'then', 'then');
                 INSERT INTO tasks (id, title, body, status, kind, product_id, priority, branch,
                                    commit_sha, verification, created_at, updated_at)
                 VALUES ('t-old', 'older than the review', 'body', 'done', 'normal', 'a/b', 4,
                         'task/t-old', 'abc1234', 'cargo test', 'then', 'then');",
            )
            .unwrap();
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        legacy.execute_batch(SCHEMA_V4).unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 4, "the fixture must start at version 4");
        let columns = column_names(&legacy, "tasks");
        assert!(
            !columns.contains(&"review_target_task_id".to_owned())
                && !columns.contains(&"review_verdict".to_owned()),
            "a version 4 database has no review columns yet"
        );
        let row_before = row_before_later_versions(&legacy, "t-old");
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        db.with_conn(|conn| {
            let columns = column_names(conn, "tasks");
            for added in ["review_target_task_id", "review_verdict"] {
                assert_eq!(
                    columns.iter().filter(|name| *name == added).count(),
                    1,
                    "{added} must be added exactly once: {columns:?}"
                );
            }
            let (target, verdict): (Option<String>, Option<String>) = conn.query_row(
                "SELECT review_target_task_id, review_verdict FROM tasks WHERE id = 't-old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert!(
                target.is_none() && verdict.is_none(),
                "a row that predates the columns is unreviewed"
            );
            let kept = row_before_later_versions(conn, "t-old");
            assert_eq!(kept, row_before, "no existing column may change");
            Ok(())
        })
        .unwrap();

        // Reopening runs the step again and must change nothing.
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        let tasks: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT count(*) FROM tasks", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(tasks, 1, "a second open changes nothing");
    }

    /// A database that predates the merge order comes across without one.
    ///
    /// Version 6 gave every merge a `merge_sequence`; version 7 takes the column
    /// away again, because nothing reads it. A version 5 database therefore runs
    /// both steps back to back and arrives with its rows byte for byte as they
    /// were — neither version 5 nor version 7 has the column, so nothing has to
    /// be excluded from the comparison for it to mean anything.
    #[test]
    fn a_version_five_database_arrives_without_the_merge_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(SCHEMA_V1).unwrap();
        legacy.execute_batch(SCHEMA_V2).unwrap();
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        legacy.execute_batch(SCHEMA_V4).unwrap();
        legacy.execute_batch(SCHEMA_V5).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO tasks (id, title, status, kind, product_id, created_at, updated_at)
                 VALUES ('t-old', 'work', 'approved', 'normal', 'a/b', 'early', 'early');
                 INSERT INTO tasks (id, title, status, kind, product_id, merge_target_task_id,
                                    branch, commit_sha, created_at, updated_at)
                 VALUES ('merge:t-old', 'landed', 'done', 'instant:merge', 'a/b', 't-old',
                         'task/t-old', 'aaa1111', 'early', 'early');
                 INSERT INTO tasks (id, title, status, kind, product_id, branch, commit_sha,
                                    created_at, updated_at)
                 VALUES ('t-live', 'more work', 'approved', 'normal', 'a/b', 'task/t-live',
                         'bbb2222', 'late', 'late');
                 INSERT INTO tasks (id, title, status, kind, product_id, merge_target_task_id,
                                    branch, commit_sha, created_at, updated_at)
                 VALUES ('merge:t-live', 'waiting', 'ready', 'instant:merge', 'a/b', 't-live',
                         'task/t-live', 'bbb2222', 'late', 'late');",
            )
            .unwrap();
        assert_eq!(
            legacy
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            5,
            "the fixture must start at version 5"
        );
        let seeded = ["t-old", "merge:t-old", "t-live", "merge:t-live"];
        let before: Vec<Vec<(String, String)>> =
            seeded.iter().map(|id| task_row(&legacy, id)).collect();
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);

        let upgraded = Connection::open(&path).unwrap();
        assert!(
            !column_names(&upgraded, "tasks").contains(&"merge_sequence".to_owned()),
            "version 7 leaves no merge order behind"
        );
        let after: Vec<Vec<(String, String)>> =
            seeded.iter().map(|id| task_row(&upgraded, id)).collect();
        assert_eq!(after, before, "every row came across unchanged");
        drop(upgraded);

        // The live merge is still the one holding its product, and work filed
        // before the upgrade can still have a merge issued against it.
        let claimed = crate::task::claim(
            &db,
            "luna",
            &[crate::task::TaskKind::InstantMerge],
            datetime!(2026-03-04 05:06:07 UTC),
            60,
        )
        .unwrap();
        assert_eq!(
            claimed.map(|task| task.id),
            Some("merge:t-live".to_owned()),
            "the merge that was waiting is claimable after the upgrade"
        );

        // Reopening runs nothing again.
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
    }

    /// The step on its own: a database that already reached version 6 loses the
    /// column and keeps everything else.
    #[test]
    fn a_version_six_database_gives_up_the_merge_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(SCHEMA_V1).unwrap();
        legacy.execute_batch(SCHEMA_V2).unwrap();
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        legacy.execute_batch(SCHEMA_V4).unwrap();
        legacy.execute_batch(SCHEMA_V5).unwrap();
        legacy.execute_batch(SCHEMA_V6).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO tasks (id, title, status, kind, product_id, branch, commit_sha,
                                    created_at, updated_at)
                 VALUES ('t-live', 'more work', 'approved', 'normal', 'a/b', 'task/t-live',
                         'bbb2222', 'late', 'late');
                 INSERT INTO tasks (id, title, status, kind, product_id, merge_target_task_id,
                                    branch, commit_sha, merge_sequence, created_at, updated_at)
                 VALUES ('merge:t-live', 'waiting', 'ready', 'instant:merge', 'a/b', 't-live',
                         'task/t-live', 'bbb2222', 7, 'late', 'late');",
            )
            .unwrap();
        assert_eq!(
            legacy
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            6,
            "the fixture must start at version 6"
        );
        assert!(
            column_names(&legacy, "tasks").contains(&"merge_sequence".to_owned()),
            "a version 6 database still carries the order"
        );
        let before: Vec<(String, String)> = task_row(&legacy, "merge:t-live")
            .into_iter()
            .filter(|(name, _)| name != "merge_sequence")
            .collect();
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        let upgraded = Connection::open(&path).unwrap();
        assert!(
            !column_names(&upgraded, "tasks").contains(&"merge_sequence".to_owned()),
            "the column is gone"
        );
        assert_eq!(
            task_row(&upgraded, "merge:t-live"),
            before,
            "and nothing else moved"
        );
    }

    /// A `review` or `instant:merge` task left at `done` after its target
    /// shipped is a husk from before release carried subtasks along with
    /// their target. The version 9 migration sweeps it: `released`, with the
    /// target's own tag and timestamp. A subtask of a task that has not
    /// shipped yet is left exactly as it was.
    #[test]
    fn a_version_five_database_sweeps_the_husks_release_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(SCHEMA_V1).unwrap();
        legacy.execute_batch(SCHEMA_V2).unwrap();
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        legacy.execute_batch(SCHEMA_V4).unwrap();
        legacy.execute_batch(SCHEMA_V5).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO tasks (id, title, body, status, kind, product_id, release_tag,
                                    created_at, updated_at)
                 VALUES ('t-1', 'shipped work', 'body', 'released', 'normal', 'a/b', 'v1.0.0',
                         'early', 'late');
                 INSERT INTO tasks (id, title, body, status, kind, product_id,
                                    review_target_task_id, commit_sha, verification,
                                    created_at, updated_at)
                 VALUES ('review:t-1', 'reading it', 'body', 'done', 'review', 'a/b', 't-1',
                         'abc1234', 'read the diff', 'early', 'earlyish');
                 INSERT INTO tasks (id, title, body, status, kind, product_id,
                                    merge_target_task_id, commit_sha, verification,
                                    created_at, updated_at)
                 VALUES ('merge:t-1', 'landing it', 'body', 'done', 'instant:merge', 'a/b', 't-1',
                         'abc1234', 'merged onto main', 'early', 'earlyish2');
                 INSERT INTO tasks (id, title, body, status, kind, product_id,
                                    created_at, updated_at)
                 VALUES ('t-2', 'still shipping', 'body', 'merged', 'normal', 'a/b',
                         'early', 'late');
                 INSERT INTO tasks (id, title, body, status, kind, product_id,
                                    review_target_task_id, commit_sha, verification,
                                    created_at, updated_at)
                 VALUES ('review:t-2', 'reading it', 'body', 'done', 'review', 'a/b', 't-2',
                         'def5678', 'read the diff', 'early', 'early');",
            )
            .unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 5, "the fixture must start at version 5");
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        db.with_conn(|conn| {
            let husk = |id: &str| -> (String, Option<String>, String) {
                conn.query_row(
                    "SELECT status, release_tag, updated_at FROM tasks WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap()
            };
            assert_eq!(
                husk("review:t-1"),
                ("released".into(), Some("v1.0.0".into()), "late".into()),
                "a done review of a released task follows it to released, \
                 with its tag and timestamp"
            );
            assert_eq!(
                husk("merge:t-1"),
                ("released".into(), Some("v1.0.0".into()), "late".into()),
                "a landed merge of a released task follows it the same way"
            );
            assert_eq!(
                husk("review:t-2"),
                ("done".into(), None, "early".into()),
                "a review of a task that has not shipped yet is left alone"
            );
            Ok(())
        })
        .unwrap();
    }

    /// A row already sitting at `done` or past it earned that status through
    /// `done` at some point, so its `updated_at` is the closest recoverable
    /// estimate of when. A row that never reached `done` — whatever kind it
    /// is, and even one merely sidetracked to `blocked` — gets no estimate:
    /// guessing one would assert a completion that never happened.
    #[test]
    fn a_version_eight_database_backfills_done_at_for_completed_normal_work_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(SCHEMA_V1).unwrap();
        legacy.execute_batch(SCHEMA_V2).unwrap();
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        legacy.execute_batch(SCHEMA_V4).unwrap();
        legacy.execute_batch(SCHEMA_V5).unwrap();
        legacy.execute_batch(SCHEMA_V6).unwrap();
        legacy.execute_batch(SCHEMA_V7).unwrap();
        legacy.execute_batch(SCHEMA_V8).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO tasks (id, title, status, kind, created_at, updated_at)
                 VALUES ('t-draft', 'not started', 'draft', 'normal', 'then', 'then');
                 INSERT INTO tasks (id, title, status, kind, created_at, updated_at)
                 VALUES ('t-wip', 'in progress', 'wip', 'normal', 'then', 'then');
                 INSERT INTO tasks (id, title, status, kind, created_at, updated_at)
                 VALUES ('t-blocked', 'stuck, never finished', 'blocked', 'normal', 'then', 'later');
                 INSERT INTO tasks (id, title, status, kind, created_at, updated_at)
                 VALUES ('t-done', 'finished', 'done', 'normal', 'then', 'later');
                 INSERT INTO tasks (id, title, status, kind, created_at, updated_at)
                 VALUES ('t-approved', 'reviewed', 'approved', 'normal', 'then', 'later');
                 INSERT INTO tasks (id, title, status, kind, created_at, updated_at)
                 VALUES ('t-merged', 'landed', 'merged', 'normal', 'then', 'later');
                 INSERT INTO tasks (id, title, status, kind, created_at, updated_at)
                 VALUES ('t-released', 'shipped', 'released', 'normal', 'then', 'later');
                 INSERT INTO tasks (id, title, status, kind, created_at, updated_at)
                 VALUES ('review:t-done', 'a finished review of other work', 'done', 'review',
                         'then', 'later');",
            )
            .unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 8, "the fixture must start at version 8");
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);

        let done_at = |db: &Db, id: &str| -> Option<String> {
            db.with_conn(|conn| {
                Ok(
                    conn.query_row("SELECT done_at FROM tasks WHERE id = ?1", [id], |row| {
                        row.get(0)
                    })?,
                )
            })
            .unwrap()
        };

        for id in ["t-done", "t-approved", "t-merged", "t-released"] {
            assert_eq!(
                done_at(&db, id),
                Some("later".to_owned()),
                "{id} already reached done, so it is backfilled from updated_at"
            );
        }
        for id in ["t-draft", "t-wip", "t-blocked", "review:t-done"] {
            assert_eq!(
                done_at(&db, id),
                None,
                "{id} never reached done as a normal task, so no value is invented for it"
            );
        }

        // Reopening runs the step again and must change nothing.
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        assert_eq!(done_at(&db, "t-done"), Some("later".to_owned()));
    }

    /// A review that finished — approved, or sent back for changes — is over,
    /// and must not keep the next review of the same task out. That is exactly
    /// where the review index parts company with the merge index, which is why
    /// the two cannot share a column.
    #[test]
    fn one_open_review_targets_a_task_while_a_finished_one_frees_it() {
        let db = Db::open_in_memory().unwrap();
        let insert = |id: &str, status: &str, column: &str| {
            db.with_conn(|conn| {
                conn.execute(
                    &format!(
                        "INSERT INTO tasks (id, title, status, kind, {column},
                                            created_at, updated_at)
                         VALUES (?1, 'attempt', ?2, 'review', 't-target', 'now', 'now')"
                    ),
                    rusqlite::params![id, status],
                )?;
                Ok(())
            })
        };
        let finish = |id: &str, status: &str| {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE tasks SET status = ?2 WHERE id = ?1",
                    rusqlite::params![id, status],
                )?;
                Ok(())
            })
            .unwrap();
        };
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, created_at, updated_at)
                 VALUES ('t-target', 'target', 'done', 'now', 'now')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        insert("r-1", "ready", "review_target_task_id").unwrap();
        assert!(
            insert("r-2", "ready", "review_target_task_id").is_err(),
            "a second open review for the same target must be refused"
        );

        finish("r-1", "done");
        insert("r-2", "ready", "review_target_task_id")
            .expect("a finished review must not block the next one");
        finish("r-2", "cancelled");
        insert("r-3", "ready", "review_target_task_id")
            .expect("a cancelled review must not block the next one");

        // The merge index keeps its own rule: a `done` merge still owns its
        // target, because a landed merge is not an invitation to land again.
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, kind, merge_target_task_id,
                                    created_at, updated_at)
                 VALUES ('m-1', 'merge', 'done', 'instant:merge', 't-target', 'now', 'now')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let second_merge = db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, kind, merge_target_task_id,
                                    created_at, updated_at)
                 VALUES ('m-2', 'merge', 'ready', 'instant:merge', 't-target', 'now', 'now')",
                [],
            )?;
            Ok(())
        });
        assert!(
            second_merge.is_err(),
            "a landed merge still owns its target, so the two indexes differ"
        );
    }

    /// Version 10 puts the release on the task and, because landing now ends
    /// the work of a product that does not release, takes such a product's
    /// `merged` work — and the finished subtasks under it — to `released` with
    /// no tag. A releasing product's merged work is left where it is: it is
    /// stranded, and `POST /api/releases` is the handle for it.
    #[test]
    fn a_version_nine_database_gains_the_release_columns_and_settles_non_releasing_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        for batch in [SCHEMA_V1, SCHEMA_V2] {
            legacy.execute_batch(batch).unwrap();
        }
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        for batch in [
            SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8, SCHEMA_V9,
        ] {
            legacy.execute_batch(batch).unwrap();
        }
        legacy
            .execute_batch(
                "INSERT INTO products (id, repository, releases, created_at, updated_at)
                 VALUES ('a/b', 'https://example.test/a/b.git', 1, 'early', 'early'),
                        ('c/d', 'https://example.test/c/d.git', 0, 'early', 'early');
                 INSERT INTO tasks (id, title, status, kind, product_id, created_at, updated_at)
                 VALUES ('t-ships', 'still shipping', 'merged', 'normal', 'a/b', 'early', 'late'),
                        ('t-keep', 'landed for good', 'merged', 'normal', 'c/d', 'early', 'late'),
                        ('t-open', 'not landed', 'done', 'normal', 'c/d', 'early', 'late');
                 INSERT INTO tasks (id, title, status, kind, product_id, merge_target_task_id,
                                    created_at, updated_at)
                 VALUES ('merge:t-keep', 'landing it', 'done', 'instant:merge', 'c/d', 't-keep',
                         'early', 'earlyish');",
            )
            .unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 9, "the fixture must start at version 9");
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        db.with_conn(|conn| {
            let names = column_names(conn, "tasks");
            for column in LATER_COLUMNS {
                assert!(names.contains(&column.to_owned()), "{column} arrived");
            }
            let row = |id: &str| -> (String, Option<String>, String, Option<String>) {
                conn.query_row(
                    "SELECT status, release_tag, release_level, release_task_id
                     FROM tasks WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap()
            };
            assert_eq!(
                row("t-keep"),
                ("released".into(), None, "patch".into(), None),
                "merged work of a product that does not release is settled with no tag"
            );
            assert_eq!(
                row("merge:t-keep"),
                ("released".into(), None, "patch".into(), None),
                "and its landed merge follows it"
            );
            assert_eq!(
                row("t-open").0,
                "done",
                "work that has not landed is left alone"
            );
            assert_eq!(
                row("t-ships").0,
                "merged",
                "merged work of a releasing product waits for a release to be issued"
            );
            Ok(())
        })
        .unwrap();
    }

    /// Version 12 rewrites the one-open-review index so `released` no longer
    /// counts as open. Before it, a target reviewed twice (`request_changes`, then
    /// approve) could not be shipped: moving both finished reviews to
    /// `released` collided on the index and the whole release report rolled
    /// back. The rewrite has to hold for a database that already carries such a
    /// pair, and the index must still refuse two open reviews of one target.
    #[test]
    fn a_version_eleven_database_lets_two_released_reviews_share_a_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        for batch in [SCHEMA_V1, SCHEMA_V2] {
            legacy.execute_batch(batch).unwrap();
        }
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        for batch in [
            SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8, SCHEMA_V9, SCHEMA_V10,
            SCHEMA_V11,
        ] {
            legacy.execute_batch(batch).unwrap();
        }
        legacy
            .execute_batch(
                "INSERT INTO tasks (id, title, body, status, kind, product_id, release_tag,
                                    created_at, updated_at)
                 VALUES ('t-1', 'shipped twice-reviewed work', 'body', 'merged', 'normal', 'a/b',
                         NULL, 'early', 'late');
                 INSERT INTO tasks (id, title, body, status, kind, product_id,
                                    review_target_task_id, review_attempt, review_verdict,
                                    created_at, updated_at)
                 VALUES ('review:t-1', 'first', 'body', 'done', 'review', 'a/b', 't-1', 1,
                         'request_changes', 'early', 'early');
                 INSERT INTO tasks (id, title, body, status, kind, product_id,
                                    review_target_task_id, review_attempt, review_verdict,
                                    created_at, updated_at)
                 VALUES ('review:t-1~2', 'second', 'body', 'done', 'review', 'a/b', 't-1', 2,
                         'approve', 'early', 'early');",
            )
            .unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 11, "the fixture must start at version 11");
        // On the old index this is exactly the statement the release ran.
        let collided = legacy.execute(
            "UPDATE tasks SET status = 'released' WHERE review_target_task_id = 't-1'",
            [],
        );
        assert!(
            collided.is_err(),
            "the fixture must reproduce the collision"
        );
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE tasks SET status = 'released', release_tag = 'v1.0.0'
                 WHERE review_target_task_id = 't-1'",
                [],
            )
            .expect("two finished reviews of one target may both be released");
            let released: i64 = conn
                .query_row(
                    "SELECT count(*) FROM tasks WHERE review_target_task_id = 't-1'
                     AND status = 'released'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(released, 2);
            // One open review per target is still the rule.
            conn.execute(
                "INSERT INTO tasks (id, title, body, status, kind, product_id,
                                    review_target_task_id, review_attempt, created_at, updated_at)
                 VALUES ('review:t-1~3', 'third', 'body', 'ready', 'review', 'a/b', 't-1', 3,
                         'now', 'now')",
                [],
            )
            .unwrap();
            let second_open = conn.execute(
                "INSERT INTO tasks (id, title, body, status, kind, product_id,
                                    review_target_task_id, review_attempt, created_at, updated_at)
                 VALUES ('review:t-1~4', 'fourth', 'body', 'ready', 'review', 'a/b', 't-1', 4,
                         'now', 'now')",
                [],
            );
            assert!(
                second_open.is_err(),
                "two open reviews of one target are still refused"
            );
            Ok(())
        })
        .unwrap();
    }

    /// The version 10 backfill moves the `done` reviews of a merged,
    /// non-releasing target to `released`. On the version 5 index two of them
    /// for one target collided, so a database at version 9 could not be opened
    /// at all by a build that shipped the fix. The index is widened before the
    /// backfill runs.
    #[test]
    fn a_version_nine_database_with_two_reviews_of_one_target_still_upgrades() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        for batch in [SCHEMA_V1, SCHEMA_V2] {
            legacy.execute_batch(batch).unwrap();
        }
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        for batch in [
            SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8, SCHEMA_V9,
        ] {
            legacy.execute_batch(batch).unwrap();
        }
        legacy
            .execute_batch(
                "INSERT INTO products (id, repository, releases, created_at, updated_at)
                 VALUES ('c/d', 'https://example.test/c/d.git', 0, 'early', 'early');
                 INSERT INTO tasks (id, title, status, kind, product_id, created_at, updated_at)
                 VALUES ('t-keep', 'landed for good', 'merged', 'normal', 'c/d', 'early', 'late');
                 INSERT INTO tasks (id, title, status, kind, product_id,
                                    review_target_task_id, review_attempt, review_verdict,
                                    created_at, updated_at)
                 VALUES ('review:t-keep', 'first', 'done', 'review', 'c/d', 't-keep', 1,
                         'request_changes', 'early', 'early'),
                        ('review:t-keep~2', 'second', 'done', 'review', 'c/d', 't-keep', 2,
                         'approve', 'early', 'early');",
            )
            .unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 9, "the fixture must start at version 9");
        drop(legacy);

        let db = Db::open(&path).expect("the upgrade must not collide on the review index");
        assert_eq!(user_version(&db), 13);
        db.with_conn(|conn| {
            let released: i64 = conn
                .query_row(
                    "SELECT count(*) FROM tasks WHERE review_target_task_id = 't-keep'
                     AND status = 'released'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(released, 2, "both finished reviews followed their target");
            let target: String = conn
                .query_row("SELECT status FROM tasks WHERE id = 't-keep'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(target, "released");
            Ok(())
        })
        .unwrap();
    }

    /// Version 11 adds `depends_on` and nothing else: every row comes across
    /// with the column empty, because nothing that already existed waits.
    #[test]
    fn a_version_ten_database_gains_an_empty_depends_on_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqlite.db");

        let legacy = Connection::open(&path).unwrap();
        for batch in [SCHEMA_V1, SCHEMA_V2] {
            legacy.execute_batch(batch).unwrap();
        }
        super::rebuild_tasks_without_the_product_key(&legacy).unwrap();
        for batch in [
            SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8, SCHEMA_V9, SCHEMA_V10,
        ] {
            legacy.execute_batch(batch).unwrap();
        }
        legacy
            .execute_batch(
                "INSERT INTO tasks (id, title, status, kind, product_id, created_at, updated_at)
                 VALUES ('t-1', 'first', 'draft', 'normal', 'a/b', 'early', 'early');",
            )
            .unwrap();
        let before: i64 = legacy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 10, "the fixture must start at version 10");
        let row_before = task_row(&legacy, "t-1");
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 13);
        db.with_conn(|conn| {
            assert!(column_names(conn, "tasks").contains(&"depends_on".to_owned()));
            let depends_on: Option<String> = conn
                .query_row("SELECT depends_on FROM tasks WHERE id = 't-1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert!(depends_on.is_none(), "an existing row waits for nothing");
            assert_eq!(task_row(conn, "t-1"), row_before, "no other column moved");
            Ok(())
        })
        .unwrap();
    }
}
