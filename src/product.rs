use std::fs;
use std::path::Path;

use rusqlite::{Connection, Row};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::clock::format_z;
use crate::db::Db;
use crate::error::Error;

const COLUMNS: &str = "id, repository, description, releases";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Product {
    pub id: String,
    pub repository: String,
    pub description: String,
    pub releases: bool,
}

/// One entry of the seed file. Only `id` and `repository` are required; a
/// catalogue line that says nothing else means "no description, and it ships".
#[derive(Debug, Deserialize)]
struct SeedEntry {
    id: String,
    repository: String,
    #[serde(default)]
    description: String,
    #[serde(default = "releases_by_default")]
    releases: bool,
}

fn releases_by_default() -> bool {
    true
}

impl From<SeedEntry> for Product {
    fn from(entry: SeedEntry) -> Self {
        Self {
            id: entry.id,
            repository: entry.repository,
            description: entry.description,
            releases: entry.releases,
        }
    }
}

/// Insert or update a product, preserving `created_at` for an existing row.
pub fn upsert(db: &Db, product: &Product, now: OffsetDateTime) -> Result<Product, Error> {
    let stamp = format_z(now);
    db.with_conn(|conn| write(conn, product, &stamp))
}

/// Fill the catalogue from the JSON roster at `path`.
///
/// The catalogue is the register of product identity, so a deployment states it
/// once and every restart converges on the same table: each entry is upserted,
/// which updates a product that moved or was described better and keeps the
/// `created_at` of one that was already there.
///
/// Fail-closed, and in one transaction: an unreadable file, JSON that does not
/// parse, or one entry the validation refuses stops the caller with nothing
/// written. A half-seeded catalogue nobody was told about is worse than a server
/// that did not start.
pub fn seed_from_path(
    db: &Db,
    path: impl AsRef<Path>,
    now: OffsetDateTime,
) -> Result<usize, Error> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .map_err(|err| Error::Invalid(format!("products seed {}: {err}", path.display())))?;
    let entries: Vec<SeedEntry> = serde_json::from_str(&raw)
        .map_err(|err| Error::Invalid(format!("products seed {}: {err}", path.display())))?;
    let products: Vec<Product> = entries.into_iter().map(Product::from).collect();
    let stamp = format_z(now);
    db.with_tx(|tx| {
        for product in &products {
            write(tx, product, &stamp)?;
        }
        Ok(products.len())
    })
}

fn write(conn: &Connection, product: &Product, stamp: &str) -> Result<Product, Error> {
    validate(product)?;
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

/// A product id is `org/repo`, never a path. Shared with task validation.
pub(crate) fn check_product_id(name: &str, value: &str) -> Result<(), Error> {
    let invalid = || Error::Invalid(format!("invalid {name} '{value}' (org/repo, not a path)"));
    if value.contains('\\') || value.contains("..") {
        return Err(invalid());
    }
    let mut parts = value.split('/');
    let (Some(org), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(invalid());
    };
    if !segment_ok(org) || !segment_ok(repo) {
        return Err(invalid());
    }
    Ok(())
}

fn segment_ok(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::{Product, get, list, seed_from_path, upsert};
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

    #[test]
    fn a_seed_upserts_every_entry_and_keeps_created_at() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("products.json");
        std::fs::write(
            &path,
            r#"[
                 {"id": "org/repo", "repository": "https://github.com/org/repo",
                  "description": "one line", "releases": true},
                 {"id": "org/other", "repository": "https://github.com/org/other"}
               ]"#,
        )
        .unwrap();

        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            seed_from_path(&db, &path, datetime!(2026-01-01 00:00:00 UTC)).unwrap(),
            2
        );
        assert_eq!(created_at(&db, "org/repo"), "2026-01-01T00:00:00Z");
        let bare = get(&db, "org/other").unwrap();
        assert_eq!(bare.description, "", "description defaults to empty");
        assert!(bare.releases, "releases defaults to true");

        std::fs::write(
            &path,
            r#"[{"id": "org/repo", "repository": "https://github.com/org/moved",
                 "description": "corrected", "releases": false}]"#,
        )
        .unwrap();
        assert_eq!(
            seed_from_path(&db, &path, datetime!(2026-02-02 03:04:05 UTC)).unwrap(),
            1
        );
        let moved = get(&db, "org/repo").unwrap();
        assert_eq!(moved.repository, "https://github.com/org/moved");
        assert_eq!(moved.description, "corrected");
        assert!(!moved.releases);
        assert_eq!(
            created_at(&db, "org/repo"),
            "2026-01-01T00:00:00Z",
            "an upsert keeps the original created_at"
        );
        assert_eq!(
            list(&db).unwrap().len(),
            2,
            "a shorter file never removes a product"
        );
    }

    /// One bad entry stops the whole seed, and takes the good ones with it: a
    /// caller that failed must not have to guess how far the file got.
    #[test]
    fn a_seed_is_refused_whole_when_one_entry_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        let now = datetime!(2026-01-01 00:00:00 UTC);

        for (name, body) in [
            (
                "bad-id.json",
                r#"[{"id": "org/good", "repository": "https://github.com/org/good"},
                    {"id": "../etc", "repository": "https://github.com/etc"}]"#,
            ),
            (
                "no-repository.json",
                r#"[{"id": "org/good", "repository": "https://github.com/org/good"},
                    {"id": "org/blank", "repository": "  "}]"#,
            ),
            ("broken.json", "not json at all"),
            ("missing-field.json", r#"[{"id": "org/good"}]"#),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, body).unwrap();
            assert!(
                matches!(seed_from_path(&db, &path, now), Err(Error::Invalid(_))),
                "{name} must be refused"
            );
            assert!(
                list(&db).unwrap().is_empty(),
                "{name} must not have written anything"
            );
        }

        assert!(matches!(
            seed_from_path(&db, dir.path().join("nowhere.json"), now),
            Err(Error::Invalid(_))
        ));
    }
}
