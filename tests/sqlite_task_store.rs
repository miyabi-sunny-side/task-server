use task_server::db::Db;
use task_server::error::Error;
use task_server::product::{self, Product};
use task_server::task::{self, NewTask, TaskKind, TaskStatus};
use time::OffsetDateTime;
use time::macros::datetime;

const TTL: u64 = 3600;

fn product_at(id: &str, releases: bool) -> Product {
    Product {
        id: id.into(),
        repository: format!("https://github.com/{id}.git"),
        description: String::new(),
        releases,
    }
}

fn new_task(id: &str, product_id: &str, kind: TaskKind, priority: i64) -> NewTask {
    NewTask {
        id: id.into(),
        title: format!("task {id}"),
        body: String::new(),
        product_id: Some(product_id.into()),
        kind,
        priority,
    }
}

fn now() -> OffsetDateTime {
    datetime!(2026-08-15 10:00:00 UTC)
}

#[test]
fn lifecycle_survives_reopen_and_rejects_invalid_transitions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state/sqlite.db");

    let db = Db::open(&path).unwrap();
    product::upsert(&db, &product_at("household/tasks", true), now()).unwrap();
    let created = task::create(
        &db,
        &new_task("t-1", "household/tasks", TaskKind::Normal, 0),
        now(),
    )
    .unwrap();
    assert_eq!(created.status, TaskStatus::Draft);

    assert!(matches!(
        task::set_status(&db, "t-1", TaskStatus::Wip, now()),
        Err(Error::Invalid(_))
    ));

    for to in [
        TaskStatus::Ready,
        TaskStatus::Wip,
        TaskStatus::Done,
        TaskStatus::Merged,
    ] {
        let task = task::set_status(&db, "t-1", to, now()).unwrap();
        assert_eq!(task.status, to);
    }

    drop(db);
    let db = Db::open(&path).unwrap();
    let reopened = task::get(&db, "t-1").unwrap();
    assert_eq!(reopened.status, TaskStatus::Merged);
    assert_eq!(reopened.title, "task t-1");

    let released = task::set_status(&db, "t-1", TaskStatus::Released, now()).unwrap();
    assert_eq!(released.status, TaskStatus::Released);

    assert!(matches!(
        task::set_status(&db, "t-1", TaskStatus::Ready, now()),
        Err(Error::Invalid(_))
    ));

    drop(db);
    let db = Db::open(&path).unwrap();
    assert_eq!(task::get(&db, "t-1").unwrap().status, TaskStatus::Released);
}

#[test]
fn instant_merge_task_is_claimed_before_higher_priority_normal_task() {
    let db = Db::open_in_memory().unwrap();
    product::upsert(&db, &product_at("household/tasks", true), now()).unwrap();

    for new in [
        new_task("t-normal", "household/tasks", TaskKind::Normal, 100),
        new_task("t-instant", "household/tasks", TaskKind::InstantMerge, 0),
    ] {
        task::create(&db, &new, now()).unwrap();
        task::set_status(&db, &new.id, TaskStatus::Ready, now()).unwrap();
    }

    let first = task::claim(&db, "worker-a", now(), TTL).unwrap().unwrap();
    assert_eq!(first.id, "t-instant");
    assert_eq!(first.status, TaskStatus::Wip);
    assert_eq!(first.claimed_by.as_deref(), Some("worker-a"));
    assert!(first.claim_id.is_some());
    assert!(first.claim_expires_at.is_some());

    let second = task::claim(&db, "worker-b", now(), TTL).unwrap().unwrap();
    assert_eq!(second.id, "t-normal");
    assert_ne!(second.claim_id, first.claim_id);

    assert!(task::claim(&db, "worker-c", now(), TTL).unwrap().is_none());
}

#[test]
fn merged_is_terminal_for_products_that_do_not_release() {
    let db = Db::open_in_memory().unwrap();
    product::upsert(&db, &product_at("household/tasks", false), now()).unwrap();
    product::upsert(&db, &product_at("household/site", true), now()).unwrap();

    for (id, product_id) in [("t-keep", "household/tasks"), ("t-ship", "household/site")] {
        task::create(&db, &new_task(id, product_id, TaskKind::Normal, 0), now()).unwrap();
        for to in [
            TaskStatus::Ready,
            TaskStatus::Wip,
            TaskStatus::Done,
            TaskStatus::Merged,
        ] {
            task::set_status(&db, id, to, now()).unwrap();
        }
    }

    let err = task::set_status(&db, "t-keep", TaskStatus::Released, now()).unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(message) if message.contains("household/tasks")),
        "unexpected error: {err:?}"
    );
    assert_eq!(task::get(&db, "t-keep").unwrap().status, TaskStatus::Merged);

    let shipped = task::set_status(&db, "t-ship", TaskStatus::Released, now()).unwrap();
    assert_eq!(shipped.status, TaskStatus::Released);
}
