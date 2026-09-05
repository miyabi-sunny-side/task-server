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
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
pub struct Args {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub releases: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestones: Option<Value>,
}
impl Args {
    fn fields(&self) -> Value {
        let mut v = serde_json::to_value(self).expect("MCP arguments serialize");
        v.as_object_mut()
            .expect("arguments object")
            .remove("status");
        v
    }
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
    #[tool(description = "List tasks, optionally filtering lifecycle status")]
    fn task_list(&self, Parameters(a): Parameters<Args>) -> CallToolResult {
        answer(
            task::list(&self.state, a.status.as_deref())
                .map(|tasks| json!({"tasks":tasks.iter().map(task::summary).collect::<Vec<_>>()})),
        )
    }
    #[tool(description = "Read a task and its milestone evidence")]
    fn task_get(&self, Parameters(a): Parameters<Args>) -> CallToolResult {
        answer(task::card(&self.state, a.id.as_deref().unwrap_or("")))
    }
    #[tool(
        description = "Create a draft task with title, body, product_id, optional id and priority"
    )]
    fn task_create(&self, Parameters(a): Parameters<Args>) -> CallToolResult {
        answer(task::create(&self.state, a.fields()))
    }
    #[tool(
        description = "Patch a task; id identifies it and other arguments contain changed fields"
    )]
    fn task_update(&self, Parameters(a): Parameters<Args>) -> CallToolResult {
        answer(task::patch(
            &self.state,
            a.id.as_deref().unwrap_or(""),
            a.fields(),
        ))
    }
    #[tool(description = "Set lifecycle status: draft, ready, blocked, done, cancelled, dropped")]
    fn task_set_status(&self, Parameters(a): Parameters<Args>) -> CallToolResult {
        answer(task::set_status(
            &self.state,
            a.id.as_deref().unwrap_or(""),
            a.status.as_deref().unwrap_or(""),
        ))
    }
    #[tool(
        description = "Read a run/report original Markdown, task, claim, commit and checks by run id"
    )]
    fn run_get(&self, Parameters(a): Parameters<Args>) -> CallToolResult {
        answer(crate::report::get(
            &self.state,
            a.id.as_deref().unwrap_or(""),
        ))
    }
    #[tool(description = "Delete a closed task; session haystack remains")]
    fn task_delete(&self, Parameters(a): Parameters<Args>) -> CallToolResult {
        answer(task::delete(&self.state, a.id.as_deref().unwrap_or("")))
    }
    #[tool(description = "List registered products")]
    fn product_list(&self) -> CallToolResult {
        answer(
            self.state
                .store
                .list("products")
                .map(|products| json!({"products":products.iter().map(product::summary).collect::<Vec<_>>()})),
        )
    }
    #[tool(
        description = "Register a product: id is org/repo with repository, description and releases"
    )]
    fn product_register(&self, Parameters(a): Parameters<Args>) -> CallToolResult {
        answer(product::put(
            &self.state,
            a.id.as_deref().unwrap_or(""),
            a.fields(),
        ))
    }
    #[tool(description = "Rescan the configured projects directory")]
    fn product_rescan(&self) -> CallToolResult {
        answer(product::rescan(&self.state))
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
