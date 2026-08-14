use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use crate::actions::ActionTable;
use crate::clock::{Clock, SystemClock};
use crate::error::Error;
use crate::notify::{HttpNotifier, NoopNotifier, Notifier};

pub const DEFAULT_CLAIM_TTL_SECS: u64 = 3600;
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:3000";

#[derive(Clone)]
pub struct AppState {
    pub tasks_git_dir: PathBuf,
    pub outbox_dir: PathBuf,
    pub static_dir: PathBuf,
    pub worker_capability: String,
    pub allowlist: Vec<String>,
    pub csrf_token: String,
    pub allowed_origins: Vec<String>,
    pub dev_identity: Option<String>,
    pub claim_ttl_secs: u64,
    pub clock: Arc<dyn Clock>,
    pub notifier: Arc<dyn Notifier>,
    pub action_table: ActionTable,
}

impl AppState {
    #[must_use]
    pub fn for_test(tasks_git_dir: impl Into<PathBuf>) -> Self {
        let tasks_git_dir = tasks_git_dir.into();
        let outbox_dir = tasks_git_dir.join(".outbox");
        Self {
            tasks_git_dir,
            outbox_dir,
            static_dir: PathBuf::from("client"),
            worker_capability: "test-capability".into(),
            allowlist: vec!["miyabi".into()],
            csrf_token: "test-csrf".into(),
            allowed_origins: vec!["https://task-server.test".into()],
            dev_identity: None,
            claim_ttl_secs: DEFAULT_CLAIM_TTL_SECS,
            clock: Arc::new(SystemClock),
            notifier: Arc::new(NoopNotifier),
            action_table: ActionTable::default(),
        }
    }

    /// # Errors
    ///
    /// Returns `Error::Invalid` when production secrets are missing or
    /// `CLAIM_TTL_SECS` is not a number.
    pub fn from_env() -> Result<Self, Error> {
        Self::from_vars(|key| env::var(key).ok())
    }

    /// Build state from an explicit env lookup (process env or a test map).
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let production = get("TASK_SERVER_ENV").as_deref() == Some("production");
        let require = |name: &str| -> Result<String, Error> {
            get(name).ok_or_else(|| Error::Invalid(format!("{name} is required")))
        };
        let tasks_git_dir = get("TASKS_GIT_DIR").map_or_else(
            || PathBuf::from("/nonexistent-tasks-git-dir"),
            PathBuf::from,
        );
        let outbox_dir = tasks_git_dir.join(".outbox");
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
        let allowlist = allowlist_raw
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if allowlist.is_empty() {
            return Err(Error::Invalid("APP_AUTH_ALLOWLIST is empty".into()));
        }
        let csrf_token = if production {
            require("APP_CSRF_TOKEN")?
        } else {
            get("APP_CSRF_TOKEN").unwrap_or_else(|| "dev-csrf".into())
        };
        let allowed_origins: Vec<String> = if production {
            require("APP_ALLOWED_ORIGINS")?
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        } else {
            get("APP_ALLOWED_ORIGINS")
                .map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(ToOwned::to_owned)
                        .collect()
                })
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
        let action_table = match get("ACTION_TABLE_PATH") {
            Some(path) => ActionTable::load_path(path)?,
            None => ActionTable::default(),
        };
        let notifier: Arc<dyn Notifier> = match get("NTFY_URL") {
            Some(url) if !url.is_empty() => Arc::new(HttpNotifier { url }),
            _ if production => {
                return Err(Error::Invalid("NTFY_URL is required".into()));
            }
            _ => Arc::new(NoopNotifier),
        };
        Ok(Self {
            tasks_git_dir,
            outbox_dir,
            static_dir,
            worker_capability,
            allowlist,
            csrf_token,
            allowed_origins,
            dev_identity,
            claim_ttl_secs,
            clock: Arc::new(SystemClock),
            notifier,
            action_table,
        })
    }

    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    #[must_use]
    pub fn with_notifier(mut self, notifier: Arc<dyn Notifier>) -> Self {
        self.notifier = notifier;
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

    #[must_use]
    pub fn with_action_table(mut self, table: ActionTable) -> Self {
        self.action_table = table;
        self
    }
}
