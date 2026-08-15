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
    /// Bearer capability for the administrative MCP endpoint. Kept apart from
    /// `worker_capability` on purpose: a worker credential never opens CRUD.
    pub mcp_capability: String,
    /// `Host` authorities the MCP endpoints answer to. Empty means rmcp's own
    /// default, which is loopback only — the protection against a page that
    /// re-resolves its name to `127.0.0.1` and then talks to this server from
    /// its own origin. A published deployment declares its names here; nothing
    /// switches the guard off.
    pub mcp_allowed_hosts: Vec<String>,
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
            mcp_capability: "test-mcp-capability".into(),
            mcp_allowed_hosts: Vec::new(),
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
        let mcp_capability = if production {
            require("MCP_CAPABILITY")?
        } else {
            get("MCP_CAPABILITY").unwrap_or_else(|| "dev-mcp-capability".into())
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
        if mcp_capability.is_empty() {
            return Err(Error::Invalid("MCP_CAPABILITY must be non-empty".into()));
        }
        // Left empty in development, where rmcp's loopback default is exactly
        // right. In production the deployment answers to its own name, and the
        // guard is that the set of accepted authorities is declared rather than
        // open: an undeclared list would mean either refusing every real request
        // or admitting every rebound one, so production must state it.
        let mcp_allowed_hosts = if production {
            split_list(&require("APP_MCP_ALLOWED_HOSTS")?)
        } else {
            get("APP_MCP_ALLOWED_HOSTS")
                .map(|raw| split_list(&raw))
                .unwrap_or_default()
        };
        if production && mcp_allowed_hosts.is_empty() {
            return Err(Error::Invalid("APP_MCP_ALLOWED_HOSTS is empty".into()));
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
            mcp_capability,
            mcp_allowed_hosts,
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

    /// Declare the `Host` authorities the MCP endpoints answer to, replacing
    /// rmcp's loopback default.
    #[must_use]
    pub fn with_mcp_allowed_hosts(mut self, hosts: impl IntoIterator<Item = String>) -> Self {
        self.mcp_allowed_hosts = hosts.into_iter().collect();
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::AppState;
    use crate::error::Error;

    /// The production secrets a start needs before `MCP_CAPABILITY` is added.
    fn production() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("TASK_SERVER_ENV", "production"),
            ("WORKER_CAPABILITY", "worker-secret"),
            ("APP_AUTH_ALLOWLIST", "miyabi"),
            ("APP_CSRF_TOKEN", "csrf-secret"),
            ("APP_ALLOWED_ORIGINS", "https://task-server.test"),
        ])
    }

    /// A capability that opens task CRUD to an agent is a secret, so a
    /// production start without one fails closed instead of defaulting.
    #[test]
    fn production_requires_an_mcp_capability() {
        let vars = production();
        let error = AppState::from_vars(|key| vars.get(key).map(|value| (*value).to_owned()))
            .err()
            .expect("a production start without MCP_CAPABILITY must fail");
        assert!(
            matches!(&error, Error::Invalid(message) if message.contains("MCP_CAPABILITY")),
            "unexpected error: {error:?}"
        );
    }

    /// Development gets a default so a local run needs no secrets at all, and
    /// the two capabilities stay distinct: one never opens the other's tools.
    #[test]
    fn development_defaults_the_capabilities_apart() {
        let state = AppState::from_vars(|key| match key {
            "APP_DB_PATH" => Some(":memory:".to_owned()),
            _ => None,
        })
        .expect("a development start needs no secrets");
        assert_eq!(state.mcp_capability, "dev-mcp-capability");
        assert_ne!(state.mcp_capability, state.worker_capability);
        assert!(
            state.mcp_allowed_hosts.is_empty(),
            "an undeclared allowlist means rmcp's loopback default, not none"
        );
    }

    /// Behind a reverse proxy the `Host` a client sends is the deployment's own
    /// name, which the server cannot infer. The allowlist is what keeps a
    /// rebound name out, so production declares it or does not start.
    #[test]
    fn production_requires_the_mcp_host_allowlist() {
        let mut vars = production();
        vars.insert("MCP_CAPABILITY", "mcp-secret");
        let error = AppState::from_vars(|key| vars.get(key).map(|value| (*value).to_owned()))
            .err()
            .expect("a production start without APP_MCP_ALLOWED_HOSTS must fail");
        assert!(
            matches!(&error, Error::Invalid(message) if message.contains("APP_MCP_ALLOWED_HOSTS")),
            "unexpected error: {error:?}"
        );

        vars.insert(
            "APP_MCP_ALLOWED_HOSTS",
            "tasks.example.test, tasks.internal",
        );
        vars.insert("APP_DB_PATH", ":memory:");
        let state = AppState::from_vars(|key| vars.get(key).map(|value| (*value).to_owned()))
            .expect("a declared allowlist starts");
        assert_eq!(
            state.mcp_allowed_hosts,
            ["tasks.example.test", "tasks.internal"]
        );
    }

    /// An empty declaration is not a declaration: it would clear the allowlist
    /// and admit every `Host`, which is the state this variable exists to
    /// prevent.
    #[test]
    fn production_refuses_an_empty_mcp_host_allowlist() {
        let mut vars = production();
        vars.insert("MCP_CAPABILITY", "mcp-secret");
        vars.insert("APP_MCP_ALLOWED_HOSTS", " , ");
        let error = AppState::from_vars(|key| vars.get(key).map(|value| (*value).to_owned()))
            .err()
            .expect("an empty allowlist must fail");
        assert!(
            matches!(&error, Error::Invalid(message) if message.contains("APP_MCP_ALLOWED_HOSTS")),
            "unexpected error: {error:?}"
        );
    }
}
