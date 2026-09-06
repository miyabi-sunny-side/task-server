use crate::{
    Error,
    clock::{Clock, SystemClock},
    ledger::Store,
};
use std::{env, path::PathBuf, sync::Arc};
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:3000";
pub const DEFAULT_DATA_DIR: &str = "data/ledger";
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub dev_identity: Option<String>,
    pub claim_ttl_secs: u64,
    pub clock: Arc<dyn Clock>,
}
impl AppState {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(store),
            dev_identity: None,
            claim_ttl_secs: 3600,
            clock: Arc::new(SystemClock),
        }
    }
    pub fn from_env() -> Result<Self, Error> {
        Self::from_vars(|key| env::var(key).ok())
    }
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let production = get("TASK_SERVER_ENV").as_deref() == Some("production");
        let ttl = get("CLAIM_TTL_SECS")
            .unwrap_or_else(|| "3600".into())
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0 && *v <= 86400)
            .ok_or_else(|| Error::Invalid("CLAIM_TTL_SECS must be 1..86400".into()))?;
        let configured_dir = get("APP_DATA_DIR");
        if configured_dir.is_none() && get("APP_DB_PATH").is_some() {
            return Err(Error::Invalid("APP_DB_PATH is retired; migrate using bin/task-data import-sqlite and set APP_DATA_DIR".into()));
        }
        let data_dir = PathBuf::from(configured_dir.unwrap_or_else(|| DEFAULT_DATA_DIR.into()));
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
        s.dev_identity =
            (!production).then(|| get("APP_DEV_IDENTITY").unwrap_or_else(|| "miyabi".into()));
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
