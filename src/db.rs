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

/// The sqlite database that holds the control plane state.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// Open (creating if needed) the database at `path`, running migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    /// Open a private in-memory database, running migrations.
    pub fn open_in_memory() -> Result<Self, Error> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, Error> {
        // WAL is meaningless for in-memory databases; that refusal is not fatal.
        drop(conn.execute_batch("PRAGMA journal_mode=WAL;"));
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
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

    use super::{Db, SCHEMA_V1, SCHEMA_V2};

    fn user_version(db: &Db) -> i64 {
        db.with_conn(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?))
            .unwrap()
    }

    /// Every column of one task row, in schema order, as `(name, value)`. A
    /// rebuild that dropped, reordered, or blanked a column shows up here.
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
            .collect()
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
        assert_eq!(user_version(&db), 3);
        let tables: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('products', 'tasks')",
                    [],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(tables, 2);
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
        assert_eq!(user_version(&db), 3);
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
        assert_eq!(user_version(&db), 3);

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
        let rows_before: Vec<Vec<(String, String)>> = ["t-target", "merge:t-target", "t-plain"]
            .into_iter()
            .map(|id| task_row(&legacy, id))
            .collect();
        let indexes_before = index_names(&legacy);
        drop(legacy);

        let db = Db::open(&path).unwrap();
        assert_eq!(user_version(&db), 3);

        db.with_conn(|conn| {
            let rows_after: Vec<Vec<(String, String)>> = ["t-target", "merge:t-target", "t-plain"]
                .into_iter()
                .map(|id| task_row(conn, id))
                .collect();
            assert_eq!(
                rows_after, rows_before,
                "the rebuild must keep every column"
            );
            assert_eq!(
                index_names(conn),
                indexes_before,
                "the rebuild must recreate every index"
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
}
