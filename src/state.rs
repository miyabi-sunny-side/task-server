use crate::{
    Error,
    clock::{Clock, SystemClock},
    ledger::Store,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, env, path::PathBuf, sync::Arc};
pub const DEFAULT_DATA_DIR: &str = "data/ledger";

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTargets {
    pub labels: Vec<String>,
    pub default: Option<String>,
}

impl ExecutionTargets {
    fn load(path: &str) -> Result<Self, Error> {
        let invalid = |message| Error::Invalid(format!("EXECUTION_TARGETS_FILE: {message}"));
        let text = std::fs::read_to_string(path).map_err(|e| invalid(format!("{path}: {e}")))?;
        let value: serde_json::Value =
            serde_norway::from_str(&text).map_err(|e| invalid(e.to_string()))?;
        let config: Self = serde_json::from_value(value).map_err(|e| invalid(e.to_string()))?;
        let mut names = HashSet::new();
        for label in &config.labels {
            if label.trim().is_empty() || !names.insert(label) {
                return Err(invalid("labels must be nonblank, unique strings".into()));
            }
        }
        if config
            .default
            .as_ref()
            .is_some_and(|value| !names.contains(value))
        {
            return Err(invalid("default must be in labels".into()));
        }
        Ok(config)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub claim_ttl_secs: u64,
    pub clock: Arc<dyn Clock>,
    pub execution_targets: Arc<ExecutionTargets>,
}
impl AppState {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(store),
            claim_ttl_secs: 3600,
            clock: Arc::new(SystemClock),
            execution_targets: Arc::default(),
        }
    }
    pub fn from_env() -> Result<Self, Error> {
        if matches!(
            env::var("EXECUTION_TARGETS_FILE"),
            Err(env::VarError::NotUnicode(_))
        ) {
            return Err(Error::Invalid(
                "EXECUTION_TARGETS_FILE must be Unicode".into(),
            ));
        }
        Self::from_vars(|key| env::var(key).ok())
    }
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let execution_targets = get("EXECUTION_TARGETS_FILE")
            .map(|path| ExecutionTargets::load(&path))
            .transpose()?
            .unwrap_or_default();
        let ttl = get("CLAIM_TTL_SECS")
            .unwrap_or_else(|| "3600".into())
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0 && *v <= 86400)
            .ok_or_else(|| Error::Invalid("CLAIM_TTL_SECS must be 1..86400".into()))?;
        let data_dir =
            PathBuf::from(get("APP_DATA_DIR").unwrap_or_else(|| DEFAULT_DATA_DIR.into()));
        let empty = match std::fs::read_dir(&data_dir) {
            Ok(mut entries) => entries.next().transpose()?.is_none(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => return Err(e.into()),
        };
        if empty
            && data_dir
                .parent()
                .is_some_and(|parent| parent.join("task-server.db").exists())
        {
            return Err(Error::Invalid("legacy task-server.db exists beside an empty ledger; run bin/task-data import-sqlite before starting".into()));
        }
        let mut s = Self::new(Store::open(data_dir)?);
        s.claim_ttl_secs = ttl;
        s.execution_targets = Arc::new(execution_targets);
        Ok(s)
    }
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
    #[must_use]
    pub fn with_ttl(mut self, secs: u64) -> Self {
        self.claim_ttl_secs = secs;
        self
    }
}
