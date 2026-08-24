//! The catalogue is derived from the project tree, not curated by hand.
//!
//! A roster maintained by a human drifts: `miyabisun/mux` sat in the catalogue
//! long after the clone was deleted. So a start walks `<org>/<repo>` and
//! reconciles — and the whole point of reconciling rather than upserting is that
//! a start which found nothing new writes nothing at all.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tempfile::TempDir;
use time::OffsetDateTime;
use time::macros::datetime;

use task_server::db::Db;
use task_server::error::Error;
use task_server::{product, scan};

fn first() -> OffsetDateTime {
    datetime!(2026-08-15 10:00:00 UTC)
}

fn later() -> OffsetDateTime {
    datetime!(2026-09-16 11:22:33 UTC)
}

/// A repository as a clone leaves it on disk: a `.git` directory git itself
/// would recognise — a `HEAD` naming a ref, an object store, a `refs` directory —
/// whose config names `origin`, plus whatever else the caller writes into it.
///
/// The structure is part of the fixture because it is part of the definition: a
/// directory holding nothing but a `config` is not a clone, and the walk has to
/// refuse it.
fn clone_at(root: &Path, id: &str, origin: &str) -> PathBuf {
    let dir = root.join(id);
    let git = dir.join(".git");
    fs::create_dir_all(git.join("objects")).expect("object store");
    fs::create_dir_all(git.join("refs")).expect("refs");
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
    fs::write(
        git.join("config"),
        format!("[remote \"origin\"]\n\turl = {origin}\n"),
    )
    .expect("git config");
    dir
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
    fs::write(path, body).expect("write");
}

/// The timestamps and the archive mark are not on the API surface, so the test
/// reads them from the file the server writes.
fn stamps(dir: &TempDir) -> Vec<(String, String, String, Option<String>)> {
    let conn = Connection::open(dir.path().join("state/task-server.db")).expect("open db file");
    let mut statement = conn
        .prepare("SELECT id, created_at, updated_at, archived_at FROM products ORDER BY id")
        .expect("prepare");
    let rows = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("query");
    rows.collect::<Result<Vec<_>, _>>().expect("rows")
}

/// A tree of three products, one of each shape the derivation has to cover.
fn tree(root: &Path) {
    let viewer = clone_at(
        root,
        "sunny-side/viewer",
        "git@github.com:miyabisun/viewer.git",
    );
    write(&viewer.join("README.md"), "# a viewer of things\n\nbody\n");
    write(&viewer.join(".github/workflows/release.yml"), "on: push\n");

    // Tagged, and with no `.github/workflows`: a version was cut here by hand,
    // and nothing in the repository builds a release.
    let helper = clone_at(
        root,
        "sunny-side/helper",
        "https://github.com/org/helper.git",
    );
    write(
        &helper.join(".git/packed-refs"),
        "abc123 refs/tags/v1.4.0\n",
    );

    let notes = clone_at(
        root,
        "household/notes",
        "git@github.com:miyabisun/notes.git",
    );
    write(&notes.join("README.md"), "prose with no heading\n");
}

/// The whole path a start takes, twice. The first walk registers the tree with
/// every field derived from a file the clone already has; the second, over a tree
/// that has not moved, must not write a single row — no fresh `updated_at`, no
/// upsert that advances the database's sequences for nothing.
#[test]
fn deriving_the_catalogue_twice_writes_nothing_the_second_time() {
    let dir = TempDir::new().expect("tempdir");
    let db = Db::open(dir.path().join("state/task-server.db")).expect("open db");
    let root = dir.path().join("projects");
    fs::create_dir_all(&root).expect("root");
    tree(&root);

    let scanned = scan::scan(&root).expect("walk the tree");
    assert!(scanned.skipped.is_empty(), "{:?}", scanned.skipped);
    let report = product::reconcile(&db, &scanned.products, first()).expect("reconcile");
    assert_eq!(report.inserted, 3);
    assert_eq!(report.unchanged, 0);
    assert!(report.archived.is_empty());

    let listed = product::list(&db).expect("list");
    let ids: Vec<&str> = listed.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        ["household/notes", "sunny-side/helper", "sunny-side/viewer"]
    );

    // The id is where the product sits locally; the repository is where git
    // pushes it. They disagree here on purpose, and the local placement wins the
    // id: `sunny-side/viewer` is pushed to a `miyabisun` remote.
    let viewer = product::get(&db, "sunny-side/viewer").expect("viewer");
    assert_eq!(viewer.repository, "https://github.com/miyabisun/viewer");
    assert_eq!(viewer.description, "a viewer of things");
    assert!(viewer.releases, "a workflows directory means it releases");

    let helper = product::get(&db, "sunny-side/helper").expect("helper");
    assert_eq!(helper.repository, "https://github.com/org/helper");
    assert_eq!(helper.description, "", "no README is an empty description");
    assert!(
        !helper.releases,
        "a tag is not a release pipeline: nothing here builds anything"
    );

    let notes = product::get(&db, "household/notes").expect("notes");
    assert_eq!(notes.description, "prose with no heading");
    assert!(!notes.releases, "no workflows directory means it does not");

    let after_first = stamps(&dir);
    assert_eq!(
        after_first
            .iter()
            .map(|(_, created, updated, archived)| (
                created.as_str(),
                updated.as_str(),
                archived.is_none()
            ))
            .collect::<Vec<_>>(),
        [
            ("2026-08-15T10:00:00Z", "2026-08-15T10:00:00Z", true),
            ("2026-08-15T10:00:00Z", "2026-08-15T10:00:00Z", true),
            ("2026-08-15T10:00:00Z", "2026-08-15T10:00:00Z", true),
        ]
    );

    // The restart, with the tree untouched.
    let scanned = scan::scan(&root).expect("walk again");
    let report = product::reconcile(&db, &scanned.products, later()).expect("reconcile again");
    assert_eq!(report.unchanged, 3);
    assert_eq!(report.inserted, 0);
    assert_eq!(report.updated, 0);
    assert!(report.archived.is_empty());
    assert!(report.unarchived.is_empty());
    assert_eq!(
        stamps(&dir),
        after_first,
        "an unchanged tree must leave updated_at exactly where it was"
    );
    assert_eq!(product::list(&db).expect("list"), listed);
}

/// The drift this replaces, and the accident that comes with fixing it wrongly.
/// A product deleted from the tree has to stop taking new work — but its row
/// stays, because the tasks that named it are the record, and `product_id` has no
/// foreign key that would have stopped a delete from stranding them. The tasks
/// left pointing at it are what an operator needs to hear. And a clone that comes
/// back is a product again on the next walk.
#[test]
fn a_product_that_left_the_tree_is_archived_and_revived_when_it_returns() {
    let dir = TempDir::new().expect("tempdir");
    let db = Db::open(dir.path().join("state/task-server.db")).expect("open db");
    let root = dir.path().join("projects");
    fs::create_dir_all(&root).expect("root");
    tree(&root);

    let scanned = scan::scan(&root).expect("walk");
    product::reconcile(&db, &scanned.products, first()).expect("reconcile");

    let conn = Connection::open(dir.path().join("state/task-server.db")).expect("open db file");
    for id in ["t-1", "t-2"] {
        conn.execute(
            "INSERT INTO tasks (id, title, status, product_id, created_at, updated_at)
             VALUES (?1, 'work', 'merged', 'household/notes', 'now', 'now')",
            [id],
        )
        .expect("task");
    }
    drop(conn);

    // The clone is gone; its org is not.
    fs::remove_dir_all(root.join("household/notes")).expect("remove clone");

    let scanned = scan::scan(&root).expect("walk after the removal");
    let report = product::reconcile(&db, &scanned.products, later()).expect("reconcile");
    assert_eq!(report.archived.len(), 1);
    assert_eq!(report.archived[0].id, "household/notes");
    assert_eq!(
        report.archived[0].tasks, 2,
        "the archive has to report how many tasks are left pointing at it"
    );
    assert_eq!(report.unchanged, 2, "the survivors are still untouched");

    let ids: Vec<(String, bool)> = product::list(&db)
        .expect("list")
        .into_iter()
        .map(|p| (p.id, p.archived))
        .collect();
    assert_eq!(
        ids,
        [
            ("household/notes".to_owned(), true),
            ("sunny-side/helper".to_owned(), false),
            ("sunny-side/viewer".to_owned(), false),
        ],
        "the archived product keeps its row and says so"
    );

    // Which is the point: the tasks that named it still resolve their product.
    let archived = product::get(&db, "household/notes").expect("the row is still there");
    assert!(archived.archived);
    assert_eq!(archived.repository, "https://github.com/miyabisun/notes");
    let conn = Connection::open(dir.path().join("state/task-server.db")).expect("open db file");
    let resolved: i64 = conn
        .query_row(
            "SELECT count(*) FROM tasks t JOIN products p ON p.id = t.product_id
             WHERE t.product_id = 'household/notes'",
            [],
            |row| row.get(0),
        )
        .expect("join");
    assert_eq!(resolved, 2, "history must still join to its product");
    drop(conn);

    // The clone comes back, and so does the product.
    let notes = clone_at(
        &root,
        "household/notes",
        "git@github.com:miyabisun/notes.git",
    );
    write(&notes.join("README.md"), "prose with no heading\n");
    let scanned = scan::scan(&root).expect("walk after the restore");
    let report = product::reconcile(&db, &scanned.products, later()).expect("reconcile");
    assert_eq!(report.unarchived, ["household/notes"]);
    assert!(report.archived.is_empty());
    assert!(
        !product::get(&db, "household/notes")
            .expect("notes")
            .archived,
        "restoring the clone is the whole remedy"
    );
}

/// Two ways a start must not act on an empty answer: a root that is not there at
/// all, and a root that walks clean but holds nothing. The first stops the
/// startup; the second archives nothing and says so.
#[test]
fn a_tree_that_says_nothing_never_archives_the_catalogue() {
    let dir = TempDir::new().expect("tempdir");
    let db = Db::open(dir.path().join("state/task-server.db")).expect("open db");
    let root = dir.path().join("projects");
    fs::create_dir_all(&root).expect("root");
    tree(&root);
    let scanned = scan::scan(&root).expect("walk");
    product::reconcile(&db, &scanned.products, first()).expect("reconcile");
    let before = stamps(&dir);

    // An unmounted root is a misconfiguration the startup refuses, so no
    // reconcile ever runs and the catalogue is untouched.
    assert!(matches!(
        scan::scan(dir.path().join("elsewhere")),
        Err(Error::Io(_))
    ));
    assert_eq!(stamps(&dir), before);

    // An empty walk is still not an instruction to delete everything.
    let empty = dir.path().join("empty");
    fs::create_dir_all(&empty).expect("empty root");
    let scanned = scan::scan(&empty).expect("walk the empty root");
    assert!(scanned.products.is_empty());
    let report = product::reconcile(&db, &scanned.products, later()).expect("reconcile");
    assert!(
        report.skipped_archive_all,
        "an empty walk must report that it refused to archive"
    );
    assert!(report.archived.is_empty());
    assert_eq!(
        stamps(&dir),
        before,
        "every row must survive an empty walk, unmarked"
    );
}
