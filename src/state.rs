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
    pub csrf_token: String,
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
            csrf_token: "test-csrf".into(),
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
        let csrf_token = if production {
            require("APP_CSRF_TOKEN")?
        } else {
            get("APP_CSRF_TOKEN").unwrap_or_else(|| "dev-csrf".into())
        };
        if csrf_token.is_empty() {
            return Err(Error::Invalid("APP_CSRF_TOKEN must be non-empty".into()));
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
            csrf_token,
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::AppState;
    use crate::error::Error;

    /// Everything a production start needs. Network reachability bounds both
    /// MCP surfaces; the remaining secret protects browser mutation.
    fn production() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("TASK_SERVER_ENV", "production"),
            ("APP_CSRF_TOKEN", "csrf-secret"),
            ("APP_DB_PATH", ":memory:"),
        ])
    }

    /// Production refuses to start without its browser-mutation secret instead
    /// of falling back to the published development default.
    #[test]
    fn production_names_the_secret_it_is_missing() {
        let mut vars = production();
        vars.remove("APP_CSRF_TOKEN");
        let error = AppState::from_vars(|key| vars.get(key).map(|value| (*value).to_owned()))
            .err()
            .unwrap_or_else(|| panic!("a production start without APP_CSRF_TOKEN must fail"));
        assert!(
            matches!(&error, Error::Invalid(message) if message.contains("APP_CSRF_TOKEN")),
            "unexpected error: {error:?}"
        );
    }

    /// That secret is the whole production contract. A start holding it needs
    /// no MCP or worker secret, identities, origins, or hosts in the process.
    #[test]
    fn production_starts_on_its_secret_alone() {
        let vars = production();
        let state = AppState::from_vars(|key| vars.get(key).map(|value| (*value).to_owned()))
            .expect("the remaining secret is the whole process contract");
        assert_eq!(state.csrf_token, "csrf-secret");
        assert!(
            state.dev_identity.is_none(),
            "production must not mint an identity of its own"
        );
    }

    /// Development gets a default so a local run needs no secret at all.
    #[test]
    fn development_defaults_the_remaining_secret() {
        let state = AppState::from_vars(|key| match key {
            "APP_DB_PATH" => Some(":memory:".to_owned()),
            _ => None,
        })
        .expect("a development start needs no secrets");
        assert_eq!(state.csrf_token, "dev-csrf");
    }
}
