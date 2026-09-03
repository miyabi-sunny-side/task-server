use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, Row};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::clock::format_z;
use crate::db::Db;
use crate::error::Error;

/// `archived_at` is reported as a flag rather than a timestamp: when a product
/// stopped being on disk is bookkeeping, whether it may be given new work is the
/// fact every caller needs.
const COLUMNS: &str = "id, repository, description, releases, archived_at IS NOT NULL";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Product {
    pub id: String,
    pub repository: String,
    pub description: String,
    pub releases: bool,
    /// The working copy is gone, so the product answers history but takes no new
    /// work. Derived from the row, never sent in: a `PUT` cannot archive or
    /// revive a product, only the walk of the project tree can.
    #[serde(default)]
    pub archived: bool,
}

/// Insert or update a product, preserving `created_at` for an existing row.
///
/// The archive mark is left exactly as it was. An archived product may be
/// corrected — a better description, a moved remote — but a correction is not a
/// claim that its working copy came back, and reviving it here would undo the
/// one warning that stops new work being filed against a directory that is not
/// there.
pub fn upsert(db: &Db, product: &Product, now: OffsetDateTime) -> Result<Product, Error> {
    let stamp = format_z(now);
    db.with_conn(|conn| write(conn, product, &stamp))
}

/// One product the reconcile archived, and how many task rows still name it.
///
/// The count is taken inside the same transaction as the archive, so the number
/// the log reports is the number that was true at the moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archived {
    pub id: String,
    pub tasks: usize,
}

/// What one reconcile did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub inserted: usize,
    pub updated: usize,
    /// Rows that matched in every field, and so were not written at all.
    pub unchanged: usize,
    /// Products that left the tree. The rows stay; the mark goes on.
    pub archived: Vec<Archived>,
    /// Products whose working copy came back, and whose mark came off.
    pub unarchived: Vec<String>,
    /// True when the walk came back empty, so no row was archived.
    ///
    /// A property of the answer, not of the table: an empty catalogue is exactly
    /// the state in which an empty walk looks harmless, and it is the same
    /// missing mount either way. Reporting it only when there were rows to save
    /// would hide the first start after a deployment mistake.
    pub skipped_archive_all: bool,
}

/// Make the catalogue equal `desired`, writing only the rows that differ.
///
/// Reading the whole table and comparing is the point, not an optimisation. An
/// unconditional upsert per product rewrites every row on every start: each one
/// gets a fresh `updated_at` it did not earn, and the identity sequences of the
/// database advance for conflicts that were never inserts. So a row that matches
/// in all four fields is left alone, and the only statements issued are for the
/// products that actually moved.
///
/// A product that is no longer in `desired` is archived, never deleted. Its tasks
/// keep naming it — `product_id` carries no foreign key that would have stopped a
/// delete, so the row is all that keeps a merged task's product resolvable — and
/// the mark is what refuses *new* work against a directory that is not there. A
/// working copy that comes back clears the mark on the next walk.
///
/// One transaction, and an empty `desired` archives nothing: a walk that came back
/// empty is far more likely to be a missing mount than every product deleted at
/// once, and that guard is reported rather than silent.
///
/// # Errors
///
/// Returns `Error::Invalid` when a desired product is not valid or an id is
/// listed twice, before anything is written.
pub fn reconcile(
    db: &Db,
    desired: &[Product],
    now: OffsetDateTime,
) -> Result<ReconcileReport, Error> {
    let stamp = format_z(now);
    let mut wanted: BTreeMap<&str, &Product> = BTreeMap::new();
    for product in desired {
        validate(product)?;
        if wanted.insert(product.id.as_str(), product).is_some() {
            return Err(Error::Invalid(format!(
                "product '{}' is listed twice",
                product.id
            )));
        }
    }
    db.with_tx(|tx| {
        let mut report = ReconcileReport::default();
        let existing: BTreeMap<String, Product> = all(tx)?
            .into_iter()
            .map(|product| (product.id.clone(), product))
            .collect();
        for (id, product) in &wanted {
            match existing.get(*id) {
                // Equality covers the mark too, so an archived row never matches
                // a product the walk just found: it falls through to the refresh,
                // which is what revives it.
                Some(stored) if stored == *product => report.unchanged += 1,
                Some(stored) => {
                    refresh(tx, product, &stamp)?;
                    if stored.archived {
                        report.unarchived.push(product.id.clone());
                    } else {
                        report.updated += 1;
                    }
                }
                None => {
                    write(tx, product, &stamp)?;
                    report.inserted += 1;
                }
            }
        }
        let stale: Vec<Product> = existing
            .into_values()
            .filter(|product| !wanted.contains_key(product.id.as_str()))
            .collect();
        if wanted.is_empty() {
            report.skipped_archive_all = true;
        } else {
            for product in stale {
                if product.archived {
                    // Already marked. Re-stamping it would move `updated_at` for
                    // a state that did not change.
                    continue;
                }
                let tasks = count_tasks(tx, &product.id)?;
                tx.execute(
                    "UPDATE products SET archived_at = ?2, updated_at = ?2 WHERE id = ?1",
                    rusqlite::params![product.id, stamp],
                )?;
                report.archived.push(Archived {
                    id: product.id,
                    tasks,
                });
            }
        }
        Ok(report)
    })
}

/// How many task rows name this product. Reported with a removal, because the
/// rows stay behind: a task keeps its `product_id` and is refused at the `ready`
/// gate instead.
fn count_tasks(conn: &Connection, product_id: &str) -> Result<usize, Error> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM tasks WHERE product_id = ?1",
        [product_id],
        |row| row.get(0),
    )?;
    // `count(*)` is never negative, so the fallback is unreachable.
    Ok(usize::try_from(count).unwrap_or_default())
}

/// Bring an existing row up to date with a product the walk found, and clear the
/// archive mark: the walk found it, so it is on disk.
fn refresh(conn: &Connection, product: &Product, stamp: &str) -> Result<(), Error> {
    validate(product)?;
    conn.execute(
        "UPDATE products SET repository = ?2, description = ?3, releases = ?4,
           updated_at = ?5, archived_at = NULL
         WHERE id = ?1",
        rusqlite::params![
            product.id,
            product.repository,
            product.description,
            product.releases,
            stamp,
        ],
    )?;
    Ok(())
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
    db.with_conn(all)
}

fn all(conn: &Connection) -> Result<Vec<Product>, Error> {
    let sql = format!("SELECT {COLUMNS} FROM products ORDER BY id ASC");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
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
        archived: row.get(4)?,
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

    use super::{Archived, Product, get, list, reconcile, upsert};
    use crate::db::Db;
    use crate::error::Error;

    fn product(id: &str, releases: bool) -> Product {
        Product {
            id: id.into(),
            repository: format!("https://example.test/{id}.git"),
            description: "first".into(),
            releases,
            archived: false,
        }
    }

    /// Every row's stamps, including the archive mark, as the database holds
    /// them. None of the three is on the API surface.
    fn stamps(db: &Db) -> Vec<(String, String, String, Option<String>)> {
        db.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT id, created_at, updated_at, archived_at FROM products ORDER BY id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .unwrap()
    }

    /// How many rows this connection has written since it was opened. A reconcile
    /// that found nothing to do must not move this at all.
    fn rows_written(db: &Db) -> u64 {
        db.with_conn(|conn| Ok(conn.total_changes())).unwrap()
    }

    fn add_task(db: &Db, id: &str, product_id: &str) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, title, status, product_id, created_at, updated_at)
                 VALUES (?1, 'work', 'draft', ?2, 'now', 'now')",
                rusqlite::params![id, product_id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    /// The four branches of a reconcile, in one pass: a row that is already
    /// right, one that drifted, one that is new, and one whose product left the
    /// disk. The row that is already right is the point of the whole exercise —
    /// it must come out of this untouched, `updated_at` included. And the one
    /// that left is archived rather than deleted, because the tasks that named
    /// it still have to resolve.
    #[test]
    fn reconcile_writes_only_the_rows_that_differ() {
        let db = Db::open_in_memory().unwrap();
        let first = datetime!(2026-01-01 00:00:00 UTC);
        for id in ["a/keep", "a/drift", "a/gone"] {
            upsert(&db, &product(id, true), first).unwrap();
        }
        add_task(&db, "t-1", "a/gone");
        add_task(&db, "t-2", "a/gone");
        add_task(&db, "t-3", "a/keep");

        let mut drifted = product("a/drift", false);
        drifted.description = "corrected".into();
        let desired = vec![
            product("a/keep", true),
            drifted.clone(),
            product("a/new", true),
        ];

        let later = datetime!(2026-02-02 03:04:05 UTC);
        let report = reconcile(&db, &desired, later).unwrap();

        assert_eq!(report.inserted, 1);
        assert_eq!(report.updated, 1);
        assert_eq!(report.unchanged, 1);
        assert_eq!(
            report.archived,
            [Archived {
                id: "a/gone".into(),
                tasks: 2,
            }],
            "an archive reports the id and the tasks left pointing at it"
        );
        assert!(report.unarchived.is_empty());
        assert!(!report.skipped_archive_all);

        assert_eq!(
            list(&db)
                .unwrap()
                .into_iter()
                .map(|p| (p.id, p.archived))
                .collect::<Vec<_>>(),
            [
                ("a/drift".to_owned(), false),
                ("a/gone".to_owned(), true),
                ("a/keep".to_owned(), false),
                ("a/new".to_owned(), false),
            ],
            "the archived product keeps its row, and says so"
        );
        assert_eq!(get(&db, "a/drift").unwrap(), drifted);
        assert_eq!(
            stamps(&db),
            [
                (
                    "a/drift".to_owned(),
                    "2026-01-01T00:00:00Z".to_owned(),
                    "2026-02-02T03:04:05Z".to_owned(),
                    None
                ),
                (
                    "a/gone".to_owned(),
                    "2026-01-01T00:00:00Z".to_owned(),
                    "2026-02-02T03:04:05Z".to_owned(),
                    Some("2026-02-02T03:04:05Z".to_owned())
                ),
                (
                    "a/keep".to_owned(),
                    "2026-01-01T00:00:00Z".to_owned(),
                    "2026-01-01T00:00:00Z".to_owned(),
                    None
                ),
                (
                    "a/new".to_owned(),
                    "2026-02-02T03:04:05Z".to_owned(),
                    "2026-02-02T03:04:05Z".to_owned(),
                    None
                ),
            ],
            "an update keeps created_at, and an untouched row keeps every stamp"
        );

        // The tasks of an archived product are exactly why the row stays.
        let tasks: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM tasks WHERE product_id = 'a/gone'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(tasks, 2);
    }

    /// A clone that comes back is a product again. The mark is what the walk
    /// sets and clears, so restoring the directory is the whole remedy — nobody
    /// has to re-enter the product by hand.
    #[test]
    fn a_product_whose_clone_came_back_is_unarchived() {
        let db = Db::open_in_memory().unwrap();
        let desired = vec![product("a/one", true), product("a/two", true)];
        reconcile(&db, &desired, datetime!(2026-01-01 00:00:00 UTC)).unwrap();

        let gone = reconcile(&db, &desired[..1], datetime!(2026-02-02 00:00:00 UTC)).unwrap();
        assert_eq!(gone.archived.len(), 1);
        assert!(get(&db, "a/two").unwrap().archived);

        let back = reconcile(&db, &desired, datetime!(2026-03-03 03:03:03 UTC)).unwrap();
        assert_eq!(back.unarchived, ["a/two"]);
        assert_eq!(back.unchanged, 1, "the other row is still left alone");
        assert!(back.archived.is_empty());
        let revived = get(&db, "a/two").unwrap();
        assert!(!revived.archived);
        assert_eq!(
            stamps(&db)[1],
            (
                "a/two".to_owned(),
                "2026-01-01T00:00:00Z".to_owned(),
                "2026-03-03T03:03:03Z".to_owned(),
                None
            ),
            "a revival clears the mark and keeps created_at"
        );
    }

    /// The restart case, and the reason this is a reconcile rather than an
    /// upsert: a tree that has not changed must produce no writes at all — not a
    /// no-op update, not a fresh `updated_at`, nothing. That holds for an already
    /// archived row too: it stays archived, and is not re-marked.
    #[test]
    fn a_second_reconcile_of_an_unchanged_tree_writes_nothing() {
        let db = Db::open_in_memory().unwrap();
        let desired = vec![product("a/one", true), product("b/two", false)];
        reconcile(&db, &desired, datetime!(2026-01-01 00:00:00 UTC)).unwrap();
        // Leave one row archived, so the no-op has both kinds to walk past.
        reconcile(&db, &desired[..1], datetime!(2026-02-02 00:00:00 UTC)).unwrap();

        let before = stamps(&db);
        let written = rows_written(&db);

        let report = reconcile(&db, &desired[..1], datetime!(2026-03-03 03:03:03 UTC)).unwrap();
        assert_eq!(report.unchanged, 1);
        assert_eq!(report.inserted, 0);
        assert_eq!(report.updated, 0);
        assert!(
            report.archived.is_empty(),
            "a row that is already archived is not archived again"
        );
        assert!(report.unarchived.is_empty());
        assert_eq!(
            rows_written(&db),
            written,
            "an unchanged tree must not write a single row"
        );
        assert_eq!(stamps(&db), before, "no stamp may move");
    }

    /// An empty walk is far more likely to be a missing mount than every product
    /// deleted at once, so it archives nothing and says so.
    #[test]
    fn an_empty_desired_set_never_archives_the_catalogue() {
        let db = Db::open_in_memory().unwrap();
        let desired = vec![product("a/one", true)];
        reconcile(&db, &desired, datetime!(2026-01-01 00:00:00 UTC)).unwrap();
        let before = stamps(&db);
        let written = rows_written(&db);

        let report = reconcile(&db, &[], datetime!(2026-03-03 03:03:03 UTC)).unwrap();
        assert!(report.skipped_archive_all, "the refusal has to be reported");
        assert!(report.archived.is_empty());
        assert!(!get(&db, "a/one").unwrap().archived);
        assert_eq!(stamps(&db), before);
        assert_eq!(rows_written(&db), written);
    }

    /// The same empty walk, on the very start that has no rows yet. The warning
    /// belongs to the *walk*, not to what it would have destroyed: a first start
    /// whose `APP_PROJECTS_DIR` points at the wrong path, or at a mount that is
    /// not there yet, leaves an empty catalogue and every task uncatalogued, and
    /// that is precisely the start where nobody has a stale row to notice the
    /// mistake by.
    #[test]
    fn an_empty_walk_is_reported_even_when_the_catalogue_is_empty_too() {
        let db = Db::open_in_memory().unwrap();
        let written = rows_written(&db);

        let report = reconcile(&db, &[], datetime!(2026-03-03 03:03:03 UTC)).unwrap();
        assert!(
            report.skipped_archive_all,
            "an empty walk has to be reported whether or not there was a row to lose"
        );
        assert!(report.archived.is_empty());
        assert_eq!(report.inserted, 0);
        assert_eq!(list(&db).unwrap(), [], "nothing was invented either");
        assert_eq!(rows_written(&db), written);
    }

    /// One unusable entry refuses the whole reconcile, and takes its neighbours
    /// with it: a catalogue half converged, with nobody told, is worse than a
    /// startup that failed.
    #[test]
    fn one_bad_entry_refuses_the_whole_reconcile() {
        let db = Db::open_in_memory().unwrap();
        upsert(
            &db,
            &product("a/one", true),
            datetime!(2026-01-01 00:00:00 UTC),
        )
        .unwrap();
        let before = stamps(&db);
        let now = datetime!(2026-03-03 03:03:03 UTC);

        let mut blank = product("a/blank", true);
        blank.repository = "  ".into();
        let cases: Vec<Vec<Product>> = vec![
            vec![product("a/good", true), product("../etc", true)],
            vec![product("a/good", true), product("bare", true)],
            vec![product("a/good", true), blank],
            vec![product("a/twice", true), product("a/twice", false)],
        ];
        for desired in cases {
            let refused = reconcile(&db, &desired, now);
            assert!(
                matches!(refused, Err(Error::Invalid(_))),
                "{desired:?} must be refused, got {refused:?}"
            );
            assert_eq!(
                stamps(&db),
                before,
                "a refused reconcile must not have written anything"
            );
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

/// What one walk of the project tree changed, by id, plus what it skipped.
///
/// The same walk the startup runs, callable while the server is up
/// (`POST /api/products/rescan`, MCP `product_rescan`), so a new clone joins the
/// catalogue without a restart.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Derived {
    pub inserted: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: usize,
    pub archived: Vec<String>,
    pub unarchived: Vec<String>,
    /// The walk found no products, so nothing was archived (see [`reconcile`]).
    pub skipped_archive_all: bool,
    /// Entries of the tree that are not products, by reason.
    pub skipped: BTreeMap<String, usize>,
    /// Products whose `releases` could not be read and kept their previous value.
    pub releases_unknown: Vec<String>,
}

/// Walk `root` and make the catalogue equal it. A `releases` flag the walk could
/// not read keeps the value the catalogue already holds.
///
/// # Errors
/// The walk failing (an unreadable root stops here; nothing is written), or a
/// database error.
pub fn derive_from_tree(db: &Db, root: &Path, now: OffsetDateTime) -> Result<Derived, Error> {
    let scanned = crate::scan::scan(root)?;
    let skipped = scanned
        .skipped_by_reason()
        .into_iter()
        .map(|(reason, count)| (reason.to_owned(), count))
        .collect();
    let releases_unknown = scanned.releases_unknown.clone();
    let scanned =
        scanned.with_previous_releases(|id| get(db, id).ok().map(|stored| stored.releases));
    let before: BTreeMap<String, Product> = list(db)?
        .into_iter()
        .map(|product| (product.id.clone(), product))
        .collect();
    let report = reconcile(db, &scanned.products, now)?;
    let mut inserted = Vec::new();
    let mut updated = Vec::new();
    for product in &scanned.products {
        match before.get(&product.id) {
            None => inserted.push(product.id.clone()),
            Some(stored)
                if stored.repository != product.repository
                    || stored.description != product.description
                    || stored.releases != product.releases =>
            {
                updated.push(product.id.clone());
            }
            Some(_) => {}
        }
    }
    Ok(Derived {
        inserted,
        updated,
        unchanged: report.unchanged,
        archived: report.archived.into_iter().map(|a| a.id).collect(),
        unarchived: report.unarchived,
        skipped_archive_all: report.skipped_archive_all,
        skipped,
        releases_unknown,
    })
}
