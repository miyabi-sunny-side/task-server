//! Trusted-network MCP adapters for the same Markdown domain.
use crate::{AppState, Error, product, task};
use axum::Router;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
// Separate schemas make unsupported arguments errors instead of ignored hints.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Id {
    pub id: String,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub id: String,
    pub status: String,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskList {
    #[serde(default, deserialize_with = "non_null")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "non_null")]
    pub product_id: Option<String>,
    #[serde(default, deserialize_with = "non_null")]
    pub limit: Option<usize>,
    #[serde(default, deserialize_with = "non_null")]
    pub offset: Option<usize>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductList {
    #[serde(default, deserialize_with = "non_null")]
    pub archived: Option<bool>,
    #[serde(default, deserialize_with = "non_null")]
    pub limit: Option<usize>,
    #[serde(default, deserialize_with = "non_null")]
    pub offset: Option<usize>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunList {
    #[serde(default, deserialize_with = "non_null")]
    pub task_id: Option<String>,
    #[serde(default, deserialize_with = "non_null")]
    pub product_id: Option<String>,
    #[serde(default, deserialize_with = "non_null")]
    pub source: Option<String>,
    #[serde(default, deserialize_with = "non_null")]
    pub unread: Option<bool>,
    #[serde(default, deserialize_with = "non_null")]
    pub limit: Option<usize>,
    #[serde(default, deserialize_with = "non_null")]
    pub offset: Option<usize>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub id: String,
    #[serde(default, deserialize_with = "non_null")]
    pub limit: Option<usize>,
    #[serde(default, deserialize_with = "non_null")]
    pub offset: Option<usize>,
}
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCreate {
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub id: Option<String>,
    pub title: String,
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub body: Option<String>,
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub product_id: Option<String>,
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub priority: Option<i64>,
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub depends_on: Option<String>,
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub release_level: Option<String>,
}
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskUpdate {
    pub id: String,
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub title: Option<String>,
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub body: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<String>")]
    pub product_id: Option<Value>,
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub priority: Option<i64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<String>")]
    pub depends_on: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<String>")]
    pub release_level: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<String>")]
    pub commit_sha: Option<Value>,
    #[serde(
        default,
        deserialize_with = "non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub milestones: Option<Vec<Value>>,
}
fn non_null<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}
fn fields<T: Serialize>(a: &T) -> Value {
    serde_json::to_value(a).expect("MCP arguments serialize")
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckpointRead {
    pub id: String,
    #[serde(default, deserialize_with = "non_null")]
    pub execution_id: Option<String>,
    #[serde(default, deserialize_with = "non_null")]
    pub limit: Option<usize>,
    #[serde(default, deserialize_with = "non_null")]
    pub offset: Option<usize>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckpointUpdate {
    pub id: String,
    pub claim_id: String,
    pub expected_revision: u64,
    #[serde(default)]
    pub set: serde_json::Map<String, Value>,
    #[serde(default)]
    pub delete_keys: Vec<String>,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductFields {
    pub id: String,
    #[serde(default, deserialize_with = "present")]
    #[schemars(with = "Option<String>")]
    pub repository: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    #[schemars(with = "Option<String>")]
    pub description: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    #[schemars(with = "Option<String>")]
    pub local_path: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    #[schemars(with = "Option<bool>")]
    pub releases: Option<Value>,
}
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}
impl ProductFields {
    fn fields(self) -> Value {
        let mut fields = serde_json::Map::new();
        for (key, value) in [
            ("repository", self.repository),
            ("description", self.description),
            ("local_path", self.local_path),
            ("releases", self.releases),
        ] {
            if let Some(value) = value {
                fields.insert(key.into(), value);
            }
        }
        Value::Object(fields)
    }
}

fn project(v: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|k| v.get(*k).map(|v| ((*k).into(), v.clone())))
            .collect(),
    )
}
fn brief_task(t: &Value) -> Value {
    project(
        t,
        &[
            "id",
            "product_id",
            "title",
            "status",
            "priority",
            "depends_on",
            "dependency_status",
            "blocked_by",
        ],
    )
}
fn task_receipt(t: &Value, fields: &Value) -> Value {
    let mut result = brief_task(t);
    result["ok"] = json!(true);
    result["updated_at"] = t["updated_at"].clone();
    result["changed"] = json!(
        fields
            .as_object()
            .expect("fields object")
            .keys()
            .filter(|k| *k != "id")
            .collect::<Vec<_>>()
    );
    result
}
fn nonempty(v: &Value) -> bool {
    !v.is_null()
        && v.as_array().is_none_or(|v| !v.is_empty())
        && v.as_object().is_none_or(|v| !v.is_empty())
}
fn validate_product(id: Option<&str>) -> Result<(), Error> {
    id.map_or(Ok(()), |id| product::check_product_id("product_id", id))
}
fn validate_page(limit: Option<usize>) -> Result<(), Error> {
    if limit.is_some_and(|v| !(1..=200).contains(&v)) {
        return Err(Error::Invalid("limit must be 1..200".into()));
    }
    Ok(())
}
fn page(
    key: &str,
    values: Vec<Value>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Value, Error> {
    validate_page(limit)?;
    let total = values.len();
    let offset = offset.unwrap_or(0).min(total);
    let limit = limit.unwrap_or(50);
    let end = offset.saturating_add(limit).min(total);
    Ok(
        json!({key:values.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":total,"next_offset":(end < total).then_some(end)}),
    )
}

#[derive(Clone)]
struct Admin {
    state: AppState,
    tool_router: ToolRouter<Self>,
}
fn answer(r: Result<Value, Error>) -> CallToolResult {
    match r {
        Ok(v) => CallToolResult::structured(v),
        Err(e) => {
            let mut r = CallToolResult::structured(json!({"error":e.to_string(),"code":e.code()}));
            r.is_error = Some(true);
            r
        }
    }
}
#[tool_router]
impl Admin {
    fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
    #[tool(
        description = "List compact tasks filtered by status/product_id. Default excludes closed tasks. Stable priority descending then id order; limit 1..200 (default 50), offset default 0. Follow next_offset until null; pages reflect current state."
    )]
    fn task_list(&self, Parameters(a): Parameters<TaskList>) -> CallToolResult {
        answer((|| {
            validate_product(a.product_id.as_deref())?;
            validate_page(a.limit)?;
            let tasks = task::list(&self.state, a.status.as_deref())?
                .into_iter()
                .filter(|t| a.product_id.as_ref().is_none_or(|p| t["product_id"] == *p))
                .map(|t| brief_task(&t))
                .collect();
            page("tasks", tasks, a.limit, a.offset)
        })())
    }
    #[tool(
        description = "Read task body and current lifecycle/claim/report references. Evidence and legacy completion prose: task_history. Report originals/haystack: run_list then run_get. Handoff values: task_checkpoint_get."
    )]
    fn task_get(&self, Parameters(a): Parameters<Id>) -> CallToolResult {
        answer(task::card(&self.state, &a.id).map(|t| {
            project(
                &t,
                &[
                    "id",
                    "title",
                    "body",
                    "product_id",
                    "status",
                    "priority",
                    "kind",
                    "depends_on",
                    "blocked_by",
                    "branch",
                    "commit_sha",
                    "release_level",
                    "release_tag",
                    "claim_id",
                    "claimed_by",
                    "claimed_at",
                    "lease_expires_at",
                    "interrupted_claim_id",
                    "last_claim_id",
                    "report_id",
                    "created_at",
                    "updated_at",
                    "done_at",
                    "closed_at",
                    "archived",
                    "available_transitions",
                    "runs_count",
                    "runs_unread",
                ],
            )
        }))
    }
    #[tool(
        description = "Read task evidence/history with limit 1..200 (default 50), offset default 0. Entries identify source fields: current_completion, last_report, milestones, milestone_history, legacy_completion, report_ids, legacy. Values are original data; overlapping historical records retain their provenance."
    )]
    fn task_history(&self, Parameters(a): Parameters<History>) -> CallToolResult {
        answer((|| {
            validate_page(a.limit)?;
            let t = task::card(&self.state, &a.id)?;
            let mut entries = vec![];
            let current = project(&t, &["summary", "verification", "checks"]);
            if current
                .as_object()
                .is_some_and(|m| m.values().any(nonempty))
            {
                entries.push(json!({"kind":"current_completion","value":current}));
            }
            for key in [
                "last_report",
                "milestones",
                "milestone_history",
                "legacy_completion",
                "report_ids",
                "legacy",
            ] {
                if let Some(items) = t[key].as_array() {
                    entries.extend(
                        items
                            .iter()
                            .enumerate()
                            .map(|(index, value)| json!({"kind":key,"index":index,"value":value})),
                    );
                } else if nonempty(&t[key]) {
                    entries.push(json!({"kind":key,"value":t[key]}));
                }
            }
            let mut result = page("entries", entries, a.limit, a.offset)?;
            result["task_id"] = json!(a.id);
            Ok(result)
        })())
    }
    #[tool(
        description = "Create a draft task with title, body, product_id, optional id/priority/dependency/release_level. Returns compact receipt; task_get reads the body."
    )]
    fn task_create(&self, Parameters(a): Parameters<TaskCreate>) -> CallToolResult {
        let fields = fields(&a);
        answer(task::create(&self.state, fields.clone()).map(|t| task_receipt(&t, &fields)))
    }
    #[tool(
        description = "Patch supplied task fields; id is required. Null clears product_id/depends_on/release_level/commit_sha. Returns compact receipt; task_get/task_history read results."
    )]
    fn task_update(&self, Parameters(a): Parameters<TaskUpdate>) -> CallToolResult {
        answer((|| {
            let fields = fields(&a);
            for key in ["product_id", "depends_on", "release_level", "commit_sha"] {
                if let Some(value) = fields.get(key)
                    && !value.is_null()
                    && !value.is_string()
                {
                    return Err(Error::Invalid(format!("{key} must be a string or null")));
                }
            }
            task::patch(&self.state, &a.id, fields.clone()).map(|t| task_receipt(&t, &fields))
        })())
    }
    #[tool(
        description = "Set lifecycle status: draft, ready, blocked, done, cancelled, dropped; returns compact receipt"
    )]
    fn task_set_status(&self, Parameters(a): Parameters<Status>) -> CallToolResult {
        answer(
            task::set_status(&self.state, &a.id, &a.status)
                .map(|t| task_receipt(&t, &json!({"status":a.status}))),
        )
    }
    #[tool(
        description = "List compact run/report metadata filtered by task_id/product_id/source/unread (true unread, false read). Numeric id ascending; limit 1..200 default 50, offset default 0. Read originals with run_get(id as string)."
    )]
    fn run_list(&self, Parameters(a): Parameters<RunList>) -> CallToolResult {
        answer((|| {
            validate_product(a.product_id.as_deref())?;
            validate_page(a.limit)?;
            task::sweep(&self.state)?;
            let mut runs = self.state.store.list("runs")?;
            runs.retain(|r| {
                a.task_id.as_ref().is_none_or(|v| r["task_id"] == *v)
                    && a.product_id.as_ref().is_none_or(|v| r["product_id"] == *v)
                    && a.source.as_ref().is_none_or(|v| r["source"] == *v)
                    && a.unread.is_none_or(|v| r["read_at"].is_null() == v)
            });
            runs.sort_by_key(|r| r["id"].as_u64().unwrap_or(0));
            page(
                "runs",
                runs.iter()
                    .map(|r| {
                        project(
                            r,
                            &[
                                "id",
                                "task_id",
                                "product_id",
                                "source",
                                "at",
                                "outcome",
                                "claim_id",
                                "commit_sha",
                                "read_at",
                            ],
                        )
                    })
                    .collect(),
                a.limit,
                a.offset,
            )
        })())
    }
    #[tool(
        description = "Read a run/report original Markdown, task, claim, commit and checks by run id"
    )]
    fn run_get(&self, Parameters(a): Parameters<Id>) -> CallToolResult {
        answer(crate::report::get(&self.state, &a.id))
    }
    #[tool(
        description = "Read handoff values by task id, optionally one execution_id; expired executions remain readable. Oldest first; limit 1..200 default 50, offset default 0. Follow next_offset until null."
    )]
    fn task_checkpoint_get(&self, Parameters(a): Parameters<CheckpointRead>) -> CallToolResult {
        answer((|| {
            validate_page(a.limit)?;
            let mut result = crate::checkpoint::get(&self.state, &a.id, a.execution_id.as_deref())?;
            let checkpoints = result["checkpoints"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let paging = page("checkpoints", checkpoints, a.limit, a.offset)?;
            result
                .as_object_mut()
                .expect("checkpoint response")
                .extend(paging.as_object().expect("page").clone());
            Ok(result)
        })())
    }
    #[tool(
        description = "Patch handoff values using live claim_id and expected_revision; set merges keys, delete_keys removes. JSON values max 32KiB/64 keys. No secrets; verify saved paths/agents before reuse. Returns task_id, execution_id, revision, updated_at and ok without echoing values. Does not change task state or lease."
    )]
    fn task_checkpoint_update(
        &self,
        Parameters(a): Parameters<CheckpointUpdate>,
    ) -> CallToolResult {
        answer(
            crate::checkpoint::update(
                &self.state,
                &a.id,
                crate::checkpoint::Patch {
                    claim_id: a.claim_id,
                    expected_revision: a.expected_revision,
                    set: a.set,
                    delete_keys: a.delete_keys,
                },
            )
            .map(|v| {
                let mut result = project(&v, &["execution_id", "revision", "updated_at"]);
                result["ok"] = json!(true);
                result["task_id"] = json!(a.id);
                result
            }),
        )
    }
    #[tool(description = "Delete a closed task; session haystack remains")]
    fn task_delete(&self, Parameters(a): Parameters<Id>) -> CallToolResult {
        answer(
            task::delete(&self.state, &a.id).map(|_| json!({"ok":true,"id":a.id,"deleted":true})),
        )
    }
    #[tool(
        description = "List registered product summaries, optional archived filter. Stable id order; limit 1..200 default 50, offset default 0. Follow next_offset; product_get reads full metadata."
    )]
    fn product_list(&self, Parameters(a): Parameters<ProductList>) -> CallToolResult {
        answer((|| {
            validate_page(a.limit)?;
            let mut products = self.state.store.list("products")?;
            products.retain(|p| {
                a.archived
                    .is_none_or(|v| (p["archived"] == true || p["archived_at"].is_string()) == v)
            });
            products.sort_by(|a, b| task::string(a, "id").cmp(task::string(b, "id")));
            page(
                "products",
                products
                    .iter()
                    .map(|p| project(p, &["id", "description", "archived", "archived_at"]))
                    .collect(),
                a.limit,
                a.offset,
            )
        })())
    }
    #[tool(
        description = "Read explicitly registered product metadata by stable id, including release policy and optional local_path"
    )]
    fn product_get(&self, Parameters(a): Parameters<Id>) -> CallToolResult {
        answer(self.state.store.get("products", &a.id))
    }
    #[tool(
        description = "Register a NEW stable org/repo id with canonical repository, optional description, local_path (absolute path or null), releases (true/false/null unknown). Existing IDs conflict; no filesystem discovery."
    )]
    fn product_register(&self, Parameters(a): Parameters<ProductFields>) -> CallToolResult {
        let id = a.id.clone();
        answer(product::put(&self.state, &id, a.fields()).map(|p| product::summary(&p)))
    }
    #[tool(
        description = "Patch existing product metadata. Omitted fields stay unchanged; local_path:null clears placement, releases:null means unknown, false forbids release. ID and archive history are immutable. Product policy is separate from authorization for a particular execution."
    )]
    fn product_update(&self, Parameters(a): Parameters<ProductFields>) -> CallToolResult {
        let id = a.id.clone();
        answer(product::update(&self.state, &id, a.fields()).map(|p| product::summary(&p)))
    }
    #[tool(
        description = "Archive a registered product explicitly, preserving its metadata and existing task history. Repeated calls are idempotent; registration never revives archived IDs."
    )]
    fn product_archive(&self, Parameters(a): Parameters<Id>) -> CallToolResult {
        answer(product::archive(&self.state, &a.id).map(|p| product::summary(&p)))
    }
}
#[tool_handler(router=self.tool_router)]
impl ServerHandler for Admin {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
        )
    }
}
pub fn endpoints<S: Clone + Send + Sync + 'static>(state: &AppState) -> Router<S> {
    let admin = Admin::new(state.clone());
    let mut router = Router::new();
    for path in ["/mcp", "/worker/mcp"] {
        let handler = admin.clone();
        let service = StreamableHttpService::new(
            move || Ok(handler.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().disable_allowed_hosts(),
        );
        router = router.nest_service(path, service);
    }
    router
}
