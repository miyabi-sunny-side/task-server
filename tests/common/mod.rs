use task_server::{AppState, ledger::Store};

pub fn state(store: Store) -> AppState {
    store
        .put(
            "settings",
            "execution_targets",
            serde_json::json!({"labels": ["forge", "field", "lab_2"], "default": "forge"}),
        )
        .unwrap();
    AppState::new(store)
}
