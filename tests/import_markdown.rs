//! The markdown queue that came before sqlite has to arrive whole: every
//! status mapped, everything the schema has no column for folded into the body,
//! and the source directory left exactly as it was.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_norway::{Mapping, Value};
use tempfile::TempDir;
use time::macros::datetime;

use task_server::clock::SharedClock;
use task_server::db::Db;
use task_server::import::{ImportError, ImportSources, import_markdown};
use task_server::task::{self, Task, TaskKind};

fn clock() -> SharedClock {
    SharedClock::at(datetime!(2026-08-16 09:30:00 UTC))
}

fn write(dir: &Path, relative: &str, body: &str) -> PathBuf {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
    fs::write(&path, body).expect("write fixture");
    path
}

/// Every file under `dir`, as `path -> bytes`. The import must leave this map
/// identical.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = fs::read(&path).expect("read file");
                files.insert(path, bytes);
            }
        }
    }
    files
}

fn tasks_by_id(db: &Db) -> BTreeMap<String, Task> {
    task::list(db)
        .expect("list tasks")
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect()
}

/// The `Imported v0.1 metadata` block, parsed back as YAML so the test checks
/// the values *and* their types, not a rendering.
fn imported_metadata(body: &str) -> Mapping {
    let marker = "---\n\n## Imported v0.1 metadata\n\n```yaml\n";
    let start = body.find(marker).expect("metadata block marker");
    let yaml = &body[start + marker.len()..];
    let end = yaml.find("```").expect("closing fence");
    match serde_norway::from_str(&yaml[..end]).expect("metadata yaml") {
        Value::Mapping(map) => map,
        other => panic!("metadata block is not a mapping: {other:?}"),
    }
}

fn metadata_str(body: &str, key: &str) -> String {
    match imported_metadata(body).get(Value::String(key.to_owned())) {
        Some(Value::String(text)) => text.clone(),
        other => panic!("metadata key {key} is not a string: {other:?}"),
    }
}

/// A live queue and an archive split over year directories, carrying every
/// v0.1 status the mapping table names.
fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let live = dir.path().join("live");
    let archive = dir.path().join("archive");
    write_live_queue(&live);
    write_archive(&archive);
    (dir, live, archive)
}

fn write_live_queue(live: &Path) {
    write(
        live,
        "t-draft.md",
        "---\n\
         title: draft work\n\
         status: draft\n\
         description: a line the schema has no column for\n\
         area: development\n\
         tags:\n\
         \x20 - one\n\
         \x20 - two\n\
         ---\n\
         # draft work\n\
         \n\
         Original body, kept verbatim.\n",
    );
    write(
        live,
        "t-ready.md",
        "---\n\
         title: ready work\n\
         status: ready\n\
         target_space: example/repo\n\
         next_action: pick it up\n\
         ---\n\
         Ready body.\n",
    );
    write(
        live,
        "t-running.md",
        "---\n\
         title: running work\n\
         status: running\n\
         product_id: example/repo\n\
         ---\n\
         Running body.\n",
    );
    write(
        live,
        "t-awaiting.md",
        "---\n\
         title: awaiting the user\n\
         status: awaiting_user\n\
         target_space: example/repo\n\
         commit_sha: abc1234\n\
         verification: cargo test\n\
         ---\n\
         Awaiting body.\n",
    );
    write(
        live,
        "t-done.md",
        "---\n\
         title: accepted work\n\
         status: done\n\
         target_space: example/repo\n\
         commit_sha: def5678\n\
         verification: cargo test\n\
         ---\n\
         Done body.\n",
    );
    write(
        live,
        "t-blocked.md",
        "---\n\
         title: blocked work\n\
         status: blocked\n\
         target_space: example/other\n\
         ---\n\
         Blocked body.\n",
    );
    // Written before `org/repo` was the convention. History, not damage.
    write(
        live,
        "t-legacy-name.md",
        "---\n\
         title: filed before the convention\n\
         status: ready\n\
         target_space: tasks\n\
         area: development\n\
         ---\n\
         Legacy name body.\n",
    );
}

fn write_archive(archive: &Path) {
    write(
        archive,
        "2025/t-release-requested.md",
        "---\n\
         title: waiting for a release\n\
         status: release_requested\n\
         target_space: example/repo\n\
         bump: patch\n\
         ---\n\
         Release requested body.\n",
    );
    write(
        archive,
        "2025/t-release-failed.md",
        "---\n\
         title: the release did not go out\n\
         status: release_failed\n\
         target_space: example/repo\n\
         release_repo: example/repo\n\
         ---\n\
         Release failed body.\n",
    );
    write(
        archive,
        "2026/t-released.md",
        "---\n\
         title: shipped work\n\
         status: released\n\
         target_space: example/repo\n\
         release_tag: v0.1.0\n\
         release_sha: 9abcdef\n\
         ---\n\
         Released body.\n",
    );
    write(
        archive,
        "2026/t-cancelled.md",
        "---\n\
         title: called off\n\
         status: cancelled\n\
         ---\n\
         Cancelled body.\n",
    );
    write(
        archive,
        "2026/t-dropped.md",
        "---\n\
         title: let go\n\
         status: dropped\n\
         ---\n\
         Dropped body.\n",
    );
    write(
        archive,
        "2026/t-legacy-path.md",
        "---\n\
         title: filed against a directory\n\
         status: done\n\
         target_space: projects/queue/tasks\n\
         ---\n\
         Legacy path body.\n",
    );
}

/// Columns keep what has one, and everything else is folded into the body
/// with its type intact.
fn assert_every_value_survived(tasks: &BTreeMap<String, Task>) {
    let draft = &tasks["t-draft"];
    assert_eq!(draft.title, "draft work");
    assert_eq!(draft.kind, TaskKind::Normal);
    assert_eq!(draft.priority, 0);
    assert!(draft.claimed_by.is_none() && draft.claim_id.is_none());
    assert!(
        draft
            .body
            .starts_with("# draft work\n\nOriginal body, kept verbatim.\n"),
        "the original body has to come first: {:?}",
        draft.body
    );
    let metadata = imported_metadata(&draft.body);
    assert_eq!(
        metadata.get(Value::String("status".into())),
        Some(&Value::String("draft".into())),
        "the pre-mapping status is always folded in, so the mapping is reversible"
    );
    assert_eq!(
        metadata.get(Value::String("description".into())),
        Some(&Value::String("a line the schema has no column for".into()))
    );
    assert_eq!(
        metadata.get(Value::String("area".into())),
        Some(&Value::String("development".into()))
    );
    assert_eq!(
        metadata.get(Value::String("tags".into())),
        Some(&Value::Sequence(vec![
            Value::String("one".into()),
            Value::String("two".into()),
        ])),
        "a list stays a list"
    );
    assert!(
        !metadata.contains_key(Value::String("title".into())),
        "a key that reached its own column is not repeated"
    );

    let awaiting = &tasks["t-awaiting"];
    assert_eq!(awaiting.product_id.as_deref(), Some("example/repo"));
    assert_eq!(awaiting.commit_sha.as_deref(), Some("abc1234"));
    assert_eq!(awaiting.verification.as_deref(), Some("cargo test"));
    assert_eq!(metadata_str(&awaiting.body, "status"), "awaiting_user");
    assert!(
        !imported_metadata(&awaiting.body).contains_key(Value::String("target_space".into())),
        "target_space reached product_id, so it is not repeated"
    );

    let running = &tasks["t-running"];
    assert_eq!(
        running.product_id.as_deref(),
        Some("example/repo"),
        "product_id is the fallback when there is no target_space"
    );

    let released = &tasks["t-released"];
    assert_eq!(released.release_tag.as_deref(), Some("v0.1.0"));
    assert_eq!(metadata_str(&released.body, "release_sha"), "9abcdef");
    assert!(
        !imported_metadata(&released.body).contains_key(Value::String("release_tag".into())),
        "release_tag reached its own column"
    );

    let cancelled = &tasks["t-cancelled"];
    assert!(cancelled.product_id.is_none());
    assert_eq!(
        imported_metadata(&cancelled.body).len(),
        1,
        "a file with nothing left over still records where it came from"
    );
    assert_eq!(metadata_str(&cancelled.body, "status"), "cancelled");
}

#[test]
fn a_live_and_archive_import_maps_every_status_and_keeps_what_it_cannot_map() {
    let (_dir, live, archive) = fixture();
    let before = snapshot(live.parent().expect("fixture root"));
    let db = Db::open_in_memory().expect("db");

    let summary = import_markdown(
        &db,
        &ImportSources {
            live: Some(live.clone()),
            archive: Some(archive.clone()),
        },
        &clock(),
    )
    .expect("import");

    assert_eq!(summary.live_files, 7);
    assert_eq!(summary.archive_files, 6);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.inserted_total(), 13);

    let tasks = tasks_by_id(&db);
    let mapped: BTreeMap<&str, &str> = tasks
        .iter()
        .map(|(id, task)| (id.as_str(), task.status.as_str()))
        .collect();
    assert_eq!(
        mapped,
        BTreeMap::from([
            ("t-draft", "draft"),
            ("t-ready", "ready"),
            ("t-running", "wip"),
            ("t-awaiting", "done"),
            ("t-done", "merged"),
            ("t-blocked", "blocked"),
            ("t-release-requested", "merged"),
            ("t-release-failed", "merged"),
            ("t-released", "released"),
            ("t-cancelled", "cancelled"),
            ("t-dropped", "dropped"),
            ("t-legacy-name", "ready"),
            ("t-legacy-path", "merged"),
        ])
    );

    assert_every_value_survived(&tasks);

    assert_eq!(
        summary.uncatalogued_products,
        ["example/other", "example/repo"],
        "an uncatalogued product is a warning, never a refusal"
    );

    let stamp = "2026-08-16T09:30:00Z";
    for task in tasks.values() {
        assert_eq!(task.created_at, stamp, "task {}", task.id);
        assert_eq!(task.updated_at, stamp, "task {}", task.id);
    }

    assert_eq!(
        snapshot(live.parent().expect("fixture root")),
        before,
        "the import must not touch the markdown it read"
    );
}

#[test]
fn one_bad_file_writes_nothing() {
    let (_dir, live, archive) = fixture();
    write(&live, "broken.md", "---\ntitle: [unclosed\n---\nbody\n");
    write(
        &live,
        "t-dup.md",
        "---\ntitle: one\nstatus: draft\n---\nbody\n",
    );
    write(
        &archive,
        "2026/t-dup.md",
        "---\ntitle: another\nstatus: draft\n---\nbody\n",
    );
    write(
        &live,
        "t-no-title.md",
        "---\nstatus: draft\ntarget_space: example/repo\n---\nbody\n",
    );
    write(
        &live,
        "t-unknown-status.md",
        "---\ntitle: unknown status\nstatus: frobnicated\n---\nbody\n",
    );

    let db = Db::open_in_memory().expect("db");
    let error = import_markdown(
        &db,
        &ImportSources {
            live: Some(live.clone()),
            archive: Some(archive.clone()),
        },
        &clock(),
    )
    .expect_err("a bad file must refuse the whole import");

    let ImportError::Refused(problems) = &error else {
        panic!("expected a refusal listing every bad file, got {error:?}");
    };
    let report = error.to_string();
    for name in [
        "broken.md",
        "t-dup.md",
        "t-no-title.md",
        "t-unknown-status.md",
    ] {
        assert!(
            problems
                .iter()
                .any(|problem| problem.path.to_string_lossy().contains(name)),
            "{name} must be named in the report: {problems:?}"
        );
        assert!(report.contains(name), "{name} must be printed: {report}");
    }
    assert!(
        report.contains("frobnicated"),
        "the reason has to say what was wrong: {report}"
    );
    assert!(
        !report.contains("t-legacy"),
        "a product reference from before the convention is not a bad file: {report}"
    );

    assert!(
        task::list(&db).expect("list tasks").is_empty(),
        "one bad file means nothing is written"
    );
}

/// A product reference that predates `org/repo` is history, not damage. The
/// import keeps the row and the value it carried; deciding the real product is
/// a human's job, and the `ready` gate is where it gets asked for. Refusing the
/// migration over it would mean rewriting an archive that v0.2.0 deliberately
/// leaves alone.
#[test]
fn a_product_reference_from_before_the_convention_never_stops_the_import() {
    let (_dir, live, archive) = fixture();
    let before = snapshot(live.parent().expect("fixture root"));
    let db = Db::open_in_memory().expect("db");

    let summary = import_markdown(
        &db,
        &ImportSources {
            live: Some(live.clone()),
            archive: Some(archive.clone()),
        },
        &clock(),
    )
    .expect("a legacy product reference must not refuse the import");

    let tasks = tasks_by_id(&db);
    for (id, original) in [
        ("t-legacy-name", "tasks"),
        ("t-legacy-path", "projects/queue/tasks"),
    ] {
        let task = &tasks[id];
        assert!(
            task.product_id.is_none(),
            "{id} must arrive without a product_id: {:?}",
            task.product_id
        );
        assert_eq!(
            metadata_str(&task.body, "target_space"),
            original,
            "the value the file carried has to survive in the body"
        );
        assert_eq!(metadata_str(&task.body, "status"), original_status(id));
    }

    assert_eq!(
        summary.legacy_product_refs, 2,
        "the summary has to say how many rows arrived without their product"
    );
    assert!(
        !summary.uncatalogued_products.iter().any(|id| id == "tasks"),
        "a legacy reference never reaches the catalogue warning: {:?}",
        summary.uncatalogued_products
    );
    let printed = summary.to_string();
    assert!(
        printed.contains("2 task(s) kept a legacy product reference"),
        "the summary must print the count: {printed}"
    );

    assert_eq!(
        snapshot(live.parent().expect("fixture root")),
        before,
        "the import must not rewrite the markdown to fix the reference"
    );
}

fn original_status(id: &str) -> &'static str {
    match id {
        "t-legacy-name" => "ready",
        _ => "done",
    }
}

#[test]
fn re_running_skips_and_a_changed_file_aborts() {
    let (_dir, live, archive) = fixture();
    let db = Db::open_in_memory().expect("db");
    let sources = || ImportSources {
        live: Some(live.clone()),
        archive: Some(archive.clone()),
    };

    let first = import_markdown(&db, &sources(), &clock()).expect("first import");
    assert_eq!(first.inserted_total(), 13);
    let after_first = tasks_by_id(&db);

    let later = SharedClock::at(datetime!(2026-09-17 12:00:00 UTC));
    let second = import_markdown(&db, &sources(), &later).expect("a repeat import is a no-op");
    assert_eq!(second.skipped, 13, "an identical file is skipped");
    assert_eq!(second.inserted_total(), 0);
    assert_eq!(
        tasks_by_id(&db),
        after_first,
        "a repeat import must not rewrite a single row"
    );

    write(
        &live,
        "t-draft.md",
        "---\ntitle: the title changed\nstatus: draft\n---\nA different body.\n",
    );
    write(
        &live,
        "t-new.md",
        "---\ntitle: never imported before\nstatus: draft\n---\nNew body.\n",
    );

    let error =
        import_markdown(&db, &sources(), &later).expect_err("a changed file must be a conflict");
    let ImportError::Refused(problems) = &error else {
        panic!("expected a conflict refusal, got {error:?}");
    };
    assert!(
        problems
            .iter()
            .any(|problem| problem.path.to_string_lossy().contains("t-draft.md")),
        "the conflicting file must be named: {problems:?}"
    );
    assert_eq!(
        tasks_by_id(&db),
        after_first,
        "a conflict leaves the database exactly as it was"
    );
    assert!(
        !tasks_by_id(&db).contains_key("t-new"),
        "the row this run would have added must not survive the abort"
    );
}

#[test]
fn sources_are_parsed_from_arguments_and_at_least_one_is_required() {
    let args =
        |items: &[&str]| -> Vec<String> { items.iter().map(|item| (*item).to_owned()).collect() };

    let sources = ImportSources::from_args(&args(&["--live", "queue", "--archive", "old"]))
        .expect("both directories");
    assert_eq!(sources.live.as_deref(), Some(Path::new("queue")));
    assert_eq!(sources.archive.as_deref(), Some(Path::new("old")));

    let archive_only =
        ImportSources::from_args(&args(&["--archive", "old"])).expect("archive alone is enough");
    assert!(archive_only.live.is_none());

    assert!(
        ImportSources::from_args(&args(&[])).is_err(),
        "both omitted is an error"
    );
    assert!(
        ImportSources::from_args(&args(&["--live"])).is_err(),
        "a flag without a value is an error"
    );
    assert!(
        ImportSources::from_args(&args(&["--nope", "x"])).is_err(),
        "an unknown flag is an error"
    );
}

/// A directory that is not there is a refusal like any other, and it names the
/// path so the operator can see the typo.
#[test]
fn a_missing_directory_is_reported_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Db::open_in_memory().expect("db");
    let missing = dir.path().join("nowhere");

    let error = import_markdown(
        &db,
        &ImportSources {
            live: Some(missing.clone()),
            archive: None,
        },
        &clock(),
    )
    .expect_err("a missing directory must fail");
    assert!(error.to_string().contains("nowhere"), "{error}");
    assert!(task::list(&db).expect("list tasks").is_empty());
}

/// The import files ordinary work: nothing arrives claimed, and nothing arrives
/// as a merge task the control plane never issued.
#[test]
fn imported_rows_are_plain_normal_tasks() {
    let (_dir, live, _archive) = fixture();
    let db = Db::open_in_memory().expect("db");
    import_markdown(
        &db,
        &ImportSources {
            live: Some(live),
            archive: None,
        },
        &clock(),
    )
    .expect("import");

    for task in task::list(&db).expect("list tasks") {
        assert_eq!(task.kind, TaskKind::Normal, "task {}", task.id);
        assert!(task.merge_target_task_id.is_none(), "task {}", task.id);
        assert!(task.claim_expires_at.is_none(), "task {}", task.id);
        assert!(task.checks.is_empty(), "task {}", task.id);
        assert!(task.branch.is_none(), "task {}", task.id);
    }
}
