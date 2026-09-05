use serde_json::json;
use task_server::{AppState, ledger::Store, product, task};

#[test]
fn explicit_registration_patch_archive_preserve_identity_and_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::new(Store::open(dir.path()).unwrap());
    let registered = product::put(
        &s,
        "old/name",
        json!({"repository":"https://example/new/name"}),
    )
    .unwrap();
    assert!(registered["releases"].is_null());
    task::create(
        &s,
        json!({"id":"task","title":"Keep me","product_id":"old/name"}),
    )
    .unwrap();
    assert!(product::put(&s, "old/name", json!({"repository":"replacement"})).is_err());
    let original = s.store.get("products", "old/name").unwrap();
    let mut with_extra = original.clone();
    with_extra["body"] = json!("Original Markdown");
    with_extra["custom"] = json!({"keep":[1,2]});
    s.store.put("products", "old/name", with_extra).unwrap();
    let updated = product::update(
        &s,
        "old/name",
        json!({"releases":false,"local_path":"/arbitrary/location"}),
    )
    .unwrap();
    assert_eq!(updated["repository"], original["repository"]);
    assert_eq!(updated["releases"], false);
    assert_eq!(updated["body"], "Original Markdown");
    assert_eq!(updated["custom"], json!({"keep":[1,2]}));
    let archived = product::archive(&s, "old/name").unwrap();
    let patched = product::update(
        &s,
        "old/name",
        json!({"description":"Changed","releases":null,"local_path":null}),
    )
    .unwrap();
    assert_eq!(patched["archived"], true);
    assert_eq!(patched["archived_at"], archived["archived_at"]);
    assert!(patched["releases"].is_null());
    assert!(patched["local_path"].is_null());
    assert_eq!(
        s.store.get("tasks", "task").unwrap()["product_id"],
        "old/name"
    );
    assert_eq!(product::archive(&s, "old/name").unwrap(), patched);
    assert!(product::update(&s, "absent/product", json!({"releases":false})).is_err());
}

#[test]
fn product_fields_are_validated_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::new(Store::open(dir.path()).unwrap());
    product::put(
        &s,
        "org/repo",
        json!({"repository":"https://example/repo","releases":true}),
    )
    .unwrap();
    let before = s.store.get("products", "org/repo").unwrap();
    for patch in [
        json!({"releases":"false"}),
        json!({"local_path":"relative"}),
        json!({"repository":null}),
        json!({"description":false}),
        json!({"archived":false}),
        json!({"id":"new/id"}),
    ] {
        assert!(product::update(&s, "org/repo", patch).is_err());
        assert_eq!(s.store.get("products", "org/repo").unwrap(), before);
    }
}
