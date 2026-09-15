use task_server::{AppState, ledger::Store};

pub fn state(store: Store) -> AppState {
    let mut state = AppState::new(store);
    state.execution_targets = std::sync::Arc::new(
        serde_json::from_value(serde_json::json!({
            "labels": ["forge", "field", "研究 / 試行"], "default": "forge"
        }))
        .unwrap(),
    );
    state
}
