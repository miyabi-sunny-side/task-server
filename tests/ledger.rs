use serde_json::json;
use std::sync::Arc;
use task_server::ledger::Store;

#[test]
fn manual_edit_reopen_and_unknown_fields_survive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .put(
            "tasks",
            "t-1",
            json!({"id":"t-1", "title":"first", "body":"本文\r\n\n", "custom":{"nested":[1,true]}}),
        )
        .unwrap();
    let path = dir.path().join("tasks/t-1.md");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("first", "edited");
    std::fs::write(&path, text).unwrap();
    assert_eq!(store.get("tasks", "t-1").unwrap()["title"], "edited");
    store
        .update("tasks", "t-1", |v| {
            v["status"] = json!("ready");
            Ok(())
        })
        .unwrap();
    drop(store);
    let reopened = Store::open(dir.path()).unwrap();
    let value = reopened.get("tasks", "t-1").unwrap();
    assert_eq!(value["custom"], json!({"nested":[1,true]}));
    assert_eq!(value["body"], "本文\r\n\n");
}

#[test]
fn rejected_updates_leave_original_bytes_and_invalid_documents_fail() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .put("tasks", "a", json!({"id":"a","body":"hello"}))
        .unwrap();
    let path = dir.path().join("tasks/a.md");
    let original = std::fs::read(&path).unwrap();
    assert!(
        store
            .update("tasks", "a", |v| {
                *v = json!([]);
                Ok(())
            })
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(store.create("tasks", "a", json!({"id":"a"})).is_err());
    std::fs::write(&path, "not a document").unwrap();
    assert!(store.list("tasks").is_err());
}

#[test]
fn concurrent_read_modify_write_and_process_lock() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    assert!(Store::open(dir.path()).is_err());
    store
        .put("tasks", "counter", json!({"id":"counter","count":0}))
        .unwrap();
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || {
                for _ in 0..20 {
                    store
                        .transaction(|access| {
                            let mut value = access.get("tasks", "counter")?;
                            value["count"] = json!(value["count"].as_i64().unwrap() + 1);
                            access.put("tasks", "counter", value)?;
                            Ok(())
                        })
                        .unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(store.get("tasks", "counter").unwrap()["count"], 80);
}

#[test]
fn ids_are_flat_reversible_and_collections_cannot_escape() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for id in [
        "org/repo",
        "review:t~2",
        "../outside",
        "日本語",
        "100%",
        ".",
    ] {
        store.put("products", id, json!({"id":id})).unwrap();
        assert_eq!(store.get("products", id).unwrap()["id"], id);
    }
    assert_eq!(store.list("products").unwrap().len(), 6);
    assert!(store.list("../elsewhere").is_err());
    assert!(store.put("tasks", "", json!({})).is_err());
    assert!(store.put("tasks", "a", json!({"id":"b"})).is_err());
    assert_eq!(
        std::fs::read_dir(dir.path().join("products"))
            .unwrap()
            .count(),
        6
    );
}

#[test]
fn failed_mutator_and_file_replacement_leave_no_partial_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .put("runs", "42", json!({"id":42, "note":"old"}))
        .unwrap();
    assert!(
        store
            .update("runs", "42", |record| {
                record["note"] = json!("unfinished");
                Err(task_server::error::Error::Invalid("abort".into()))
            })
            .is_err()
    );
    assert_eq!(store.get("runs", "42").unwrap()["note"], "old");
    std::fs::create_dir(dir.path().join("runs/43.md")).unwrap();
    assert!(store.put("runs", "43", json!({"id":43})).is_err());
    assert!(
        std::fs::read_dir(dir.path().join("runs"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
    );
    assert_eq!(store.remove("runs", "42").unwrap()["id"], 42);
    assert!(store.get("runs", "42").is_err());
}

#[cfg(unix)]
#[test]
fn symlink_documents_and_collection_directories_are_refused() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let file = external.path().join("external.md");
    std::fs::write(&file, "---\nid: t-1\n---\noriginal").unwrap();
    symlink(&file, dir.path().join("tasks/t-1.md")).unwrap();
    assert!(store.get("tasks", "t-1").is_err());
    assert!(store.put("tasks", "t-1", json!({"id":"t-1"})).is_err());
    assert!(store.list("tasks").is_err());
    assert!(std::fs::read_to_string(file).unwrap().ends_with("original"));
    std::fs::remove_dir(dir.path().join("products")).unwrap();
    symlink(external.path(), dir.path().join("products")).unwrap();
    assert!(store.list("products").is_err());
}
