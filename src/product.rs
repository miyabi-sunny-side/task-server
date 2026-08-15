use rusqlite::{Connection, Row};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::clock::format_z;
use crate::db::Db;
use crate::error::Error;
use crate::status::check_product_id;

const COLUMNS: &str = "id, repository, description, releases";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Product {
    pub id: String,
    pub repository: String,
    pub description: String,
    pub releases: bool,
}

/// Insert or update a product, preserving `created_at` for an existing row.
pub fn upsert(db: &Db, product: &Product, now: OffsetDateTime) -> Result<Product, Error> {
    validate(product)?;
    let stamp = format_z(now);
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO products (id, repository, description, releases, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET
               repository = excluded.repository,
               description = excluded.description,
               releases = excluded.releases,
               updated_at = excluded.updated_at",
            rusqlite::params![
                product.id,
                product.repository,
                product.description,
                product.releases,
                stamp,
            ],
        )?;
        read(conn, &product.id)
    })
}

pub fn get(db: &Db, id: &str) -> Result<Product, Error> {
    db.with_conn(|conn| read(conn, id))
}

/// All products, ordered by id.
pub fn list(db: &Db) -> Result<Vec<Product>, Error> {
    db.with_conn(|conn| {
        let sql = format!("SELECT {COLUMNS} FROM products ORDER BY id ASC");
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map([], from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })
}

pub(crate) fn read(conn: &Connection, id: &str) -> Result<Product, Error> {
    let sql = format!("SELECT {COLUMNS} FROM products WHERE id = ?1");
    conn.query_row(&sql, [id], from_row)
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => Error::NotFound,
            other => other.into(),
        })
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Product> {
    Ok(Product {
        id: row.get(0)?,
        repository: row.get(1)?,
        description: row.get(2)?,
        releases: row.get(3)?,
    })
}

fn validate(product: &Product) -> Result<(), Error> {
    check_product_id("id", &product.id)?;
    if product.repository.trim().is_empty() {
        return Err(Error::Invalid("repository is required".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::{Product, get, list, upsert};
    use crate::db::Db;
    use crate::error::Error;

    fn product(id: &str, releases: bool) -> Product {
        Product {
            id: id.into(),
            repository: format!("https://example.test/{id}.git"),
            description: "first".into(),
            releases,
        }
    }

    fn created_at(db: &Db, id: &str) -> String {
        db.with_conn(|conn| {
            Ok(conn.query_row(
                "SELECT created_at FROM products WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?)
        })
        .unwrap()
    }

    #[test]
    fn upsert_keeps_created_at_and_updates_the_rest() {
        let db = Db::open_in_memory().unwrap();
        upsert(
            &db,
            &product("a/b", true),
            datetime!(2026-01-01 00:00:00 UTC),
        )
        .unwrap();
        assert_eq!(created_at(&db, "a/b"), "2026-01-01T00:00:00Z");

        let mut changed = product("a/b", false);
        changed.description = "second".into();
        let stored = upsert(&db, &changed, datetime!(2026-02-02 03:04:05 UTC)).unwrap();

        assert_eq!(stored, changed);
        assert_eq!(created_at(&db, "a/b"), "2026-01-01T00:00:00Z");
        let updated_at: String = db
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT updated_at FROM products WHERE id = ?1",
                    ["a/b"],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(updated_at, "2026-02-02T03:04:05Z");
        assert!(!get(&db, "a/b").unwrap().releases);
    }

    #[test]
    fn get_reports_not_found_and_list_is_sorted() {
        let db = Db::open_in_memory().unwrap();
        assert!(matches!(get(&db, "a/b"), Err(Error::NotFound)));

        let now = datetime!(2026-01-01 00:00:00 UTC);
        upsert(&db, &product("z/z", true), now).unwrap();
        upsert(&db, &product("a/b", true), now).unwrap();
        let ids: Vec<String> = list(&db).unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["a/b", "z/z"]);
    }

    #[test]
    fn invalid_identifiers_and_empty_repositories_are_rejected() {
        let db = Db::open_in_memory().unwrap();
        let now = datetime!(2026-01-01 00:00:00 UTC);

        assert!(matches!(
            upsert(&db, &product("../etc", true), now),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            upsert(&db, &product("bare", true), now),
            Err(Error::Invalid(_))
        ));

        let mut empty = product("a/b", true);
        empty.repository = "  ".into();
        assert!(matches!(upsert(&db, &empty, now), Err(Error::Invalid(_))));
    }
}
