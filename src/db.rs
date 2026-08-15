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
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{Db, SCHEMA_V1};

    fn user_version(db: &Db) -> i64 {
        db.with_conn(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?))
            .unwrap()
    }

    #[test]
    fn migration_creates_the_current_schema() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(user_version(&db), 2);
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
        assert_eq!(user_version(&db), 2);
        let products: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT count(*) FROM products", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(products, 1);
    }

    /// A database written by the previous release must move to version 2 with
    /// every row intact, so an upgrade is never a data migration by hand.
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
        assert_eq!(user_version(&db), 2);

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

    #[test]
    fn foreign_keys_are_enforced() {
        let db = Db::open_in_memory().unwrap();
        let result = db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, created_at, updated_at, product_id)
                 VALUES ('t-1', 'title', 'draft', 'now', 'now', 'missing/product')",
                [],
            )?;
            Ok(())
        });
        assert!(result.is_err(), "foreign key violation must be rejected");
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
