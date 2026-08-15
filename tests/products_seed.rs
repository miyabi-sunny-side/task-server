//! The product catalogue is the register of product identity, so a deployment
//! has to be able to hand the server a roster at startup and get the same table
//! every time it restarts.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;
use time::OffsetDateTime;
use time::macros::datetime;

use task_server::db::Db;
use task_server::product;

fn first() -> OffsetDateTime {
    datetime!(2026-08-15 10:00:00 UTC)
}

fn later() -> OffsetDateTime {
    datetime!(2026-09-16 11:22:33 UTC)
}

fn write_seed(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write seed");
    path
}

/// `created_at` is not on the API surface, so the test reads it from the file
/// the server writes.
fn created_at(dir: &TempDir, id: &str) -> String {
    let conn = Connection::open(dir.path().join("state/task-server.db")).expect("open db file");
    conn.query_row(
        "SELECT created_at FROM products WHERE id = ?1",
        [id],
        |row| row.get(0),
    )
    .expect("created_at")
}

#[test]
fn seeding_products_is_idempotent_and_updates_descriptions() {
    let dir = TempDir::new().expect("tempdir");
    let db = Db::open(dir.path().join("state/task-server.db")).expect("open db");

    let path = write_seed(
        dir.path(),
        "products.json",
        &json!([
            {
                "id": "org/repo",
                "repository": "https://github.com/org/repo",
                "description": "one line",
                "releases": true,
            },
            {
                "id": "org/other",
                "repository": "https://github.com/org/other",
            },
        ])
        .to_string(),
    );

    let seeded = product::seed_from_path(&db, &path, first()).expect("seed");
    assert_eq!(seeded, 2, "every entry is upserted");
    assert_eq!(
        product::list(&db)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        ["org/other", "org/repo"]
    );

    // The optional fields have documented defaults.
    let bare = product::get(&db, "org/other").unwrap();
    assert_eq!(bare.description, "", "description defaults to empty");
    assert!(bare.releases, "releases defaults to true");
    let full = product::get(&db, "org/repo").unwrap();
    assert_eq!(full.repository, "https://github.com/org/repo");
    assert_eq!(full.description, "one line");
    assert!(full.releases);

    let stamped = created_at(&dir, "org/repo");
    assert_eq!(stamped, "2026-08-15T10:00:00Z");

    // The same file again is the restart case: no duplicates, no new rows.
    let again = product::seed_from_path(&db, &path, later()).expect("reseed");
    assert_eq!(again, 2);
    assert_eq!(product::list(&db).unwrap().len(), 2, "no duplicate rows");
    assert_eq!(product::get(&db, "org/repo").unwrap(), full);

    // An edited file is the way the catalogue is corrected.
    let path = write_seed(
        dir.path(),
        "products.json",
        &json!([
            {
                "id": "org/repo",
                "repository": "https://github.com/org/repo",
                "description": "corrected",
                "releases": false,
            },
            {
                "id": "org/other",
                "repository": "https://github.com/org/other",
                "description": "also corrected",
            },
        ])
        .to_string(),
    );
    product::seed_from_path(&db, &path, later()).expect("reseed edited");

    let corrected = product::get(&db, "org/repo").unwrap();
    assert_eq!(corrected.description, "corrected");
    assert!(!corrected.releases, "releases is updated too");
    assert_eq!(
        product::get(&db, "org/other").unwrap().description,
        "also corrected"
    );
    assert_eq!(product::list(&db).unwrap().len(), 2);
    assert_eq!(
        created_at(&dir, "org/repo"),
        stamped,
        "an upsert keeps the original created_at"
    );
}

/// A seed that cannot be trusted stops the startup that asked for it. Silently
/// skipping it would leave the catalogue partly filled and nobody told.
#[test]
fn an_unusable_seed_file_is_an_error() {
    let dir = TempDir::new().expect("tempdir");
    let db = Db::open(dir.path().join("state/task-server.db")).expect("open db");

    let missing = dir.path().join("nowhere.json");
    assert!(
        product::seed_from_path(&db, &missing, first()).is_err(),
        "a missing seed file must fail"
    );

    let broken = write_seed(dir.path(), "broken.json", "[{\"id\": \"org/repo\",");
    assert!(
        product::seed_from_path(&db, &broken, first()).is_err(),
        "broken JSON must fail"
    );

    let bad_id = write_seed(
        dir.path(),
        "bad-id.json",
        &json!([
            {"id": "org/good", "repository": "https://github.com/org/good"},
            {"id": "bare", "repository": "https://github.com/bare"},
        ])
        .to_string(),
    );
    assert!(
        product::seed_from_path(&db, &bad_id, first()).is_err(),
        "an id that is not org/repo must fail"
    );
    assert!(
        product::list(&db).unwrap().is_empty(),
        "a refused seed must not have written any of its entries"
    );
}
