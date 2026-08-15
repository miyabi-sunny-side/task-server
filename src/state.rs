use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use crate::clock::{Clock, SystemClock};
use crate::db::Db;
use crate::error::Error;

pub const DEFAULT_CLAIM_TTL_SECS: u64 = 3600;
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:3000";
pub const DEFAULT_DB_PATH: &str = "data/task-server.db";

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub static_dir: PathBuf,
    pub worker_capability: String,
    pub allowlist: Vec<String>,
    pub csrf_token: String,
    pub allowed_origins: Vec<String>,
    pub dev_identity: Option<String>,
    pub claim_ttl_secs: u64,
    pub clock: Arc<dyn Clock>,
}

impl AppState {
    #[must_use]
    pub fn for_test() -> Self {
        Self {
            db: Arc::new(Db::open_in_memory().expect("in-memory database")),
            static_dir: PathBuf::from("client"),
            worker_capability: "test-capability".into(),
            allowlist: vec!["miyabi".into()],
            csrf_token: "test-csrf".into(),
            allowed_origins: vec!["https://task-server.test".into()],
            dev_identity: None,
            claim_ttl_secs: DEFAULT_CLAIM_TTL_SECS,
            clock: Arc::new(SystemClock),
        }
    }

    /// # Errors
    ///
    /// Returns `Error::Invalid` when production secrets are missing or
    /// `CLAIM_TTL_SECS` is not a number, and `Error::Db` when the database at
    /// `APP_DB_PATH` cannot be opened.
    pub fn from_env() -> Result<Self, Error> {
        Self::from_vars(|key| env::var(key).ok())
    }

    /// Build state from an explicit env lookup (process env or a test map).
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let production = get("TASK_SERVER_ENV").as_deref() == Some("production");
        let require = |name: &str| -> Result<String, Error> {
            get(name).ok_or_else(|| Error::Invalid(format!("{name} is required")))
        };
        let static_dir =
            get("APP_STATIC_DIR").map_or_else(|| PathBuf::from("client/dist"), PathBuf::from);
        let worker_capability = if production {
            require("WORKER_CAPABILITY")?
        } else {
            get("WORKER_CAPABILITY").unwrap_or_else(|| "dev-worker-capability".into())
        };
        let allowlist_raw = if production {
            require("APP_AUTH_ALLOWLIST")?
        } else {
            get("APP_AUTH_ALLOWLIST").unwrap_or_else(|| "miyabi".into())
        };
        let allowlist = split_list(&allowlist_raw);
        if allowlist.is_empty() {
            return Err(Error::Invalid("APP_AUTH_ALLOWLIST is empty".into()));
        }
        let csrf_token = if production {
            require("APP_CSRF_TOKEN")?
        } else {
            get("APP_CSRF_TOKEN").unwrap_or_else(|| "dev-csrf".into())
        };
        let allowed_origins = if production {
            split_list(&require("APP_ALLOWED_ORIGINS")?)
        } else {
            get("APP_ALLOWED_ORIGINS")
                .map(|raw| split_list(&raw))
                .unwrap_or_default()
        };
        if production && allowed_origins.is_empty() {
            return Err(Error::Invalid("APP_ALLOWED_ORIGINS is empty".into()));
        }
        if worker_capability.is_empty() || csrf_token.is_empty() {
            return Err(Error::Invalid(
                "WORKER_CAPABILITY and APP_CSRF_TOKEN must be non-empty".into(),
            ));
        }
        let dev_identity = if production {
            None
        } else {
            Some(get("APP_DEV_IDENTITY").unwrap_or_else(|| "miyabi".into()))
        };
        let claim_ttl_secs = match get("CLAIM_TTL_SECS") {
            Some(raw) => raw
                .parse()
                .map_err(|_| Error::Invalid(format!("invalid CLAIM_TTL_SECS: {raw}")))?,
            None => DEFAULT_CLAIM_TTL_SECS,
        };
        // Opened last, so a fail-closed startup never creates a database file.
        let db_path = get("APP_DB_PATH").unwrap_or_else(|| DEFAULT_DB_PATH.to_owned());
        let db = Arc::new(Db::open(db_path)?);
        Ok(Self {
            db,
            static_dir,
            worker_capability,
            allowlist,
            csrf_token,
            allowed_origins,
            dev_identity,
            claim_ttl_secs,
            clock: Arc::new(SystemClock),
        })
    }

    #[must_use]
    pub fn with_db(mut self, db: Arc<Db>) -> Self {
        self.db = db;
        self
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

    #[must_use]
    pub fn with_static_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.static_dir = dir.into();
        self
    }
}

fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
