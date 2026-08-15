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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Db;

    fn user_version(db: &Db) -> i64 {
        db.with_conn(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?))
            .unwrap()
    }

    #[test]
    fn migration_creates_schema_version_one() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(user_version(&db), 1);
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
        assert_eq!(user_version(&db), 1);
        let products: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT count(*) FROM products", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(products, 1);
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
