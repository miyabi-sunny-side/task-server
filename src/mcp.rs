//! The MCP face of the same control plane the HTTP API serves.
//!
//! Two endpoints, two capabilities, one database. `/mcp` is the administrative
//! surface a chat agent uses to file and groom work; `/worker/mcp` is the two
//! calls a worker loop needs. Both are thin adapters: every tool decodes its
//! arguments, calls [`crate::task`] or [`crate::product`], and encodes the
//! answer. No rule lives here that does not also hold for HTTP.
//!
//! A refusal from the domain is not a protocol failure — the request was well
//! formed and the server understood it — so it comes back as a tool result with
//! `isError: true` carrying `{"error", "code"}` in both `structuredContent` and
//! the text content. `code` is the same stable slug the HTTP API answers with,
//! so a model branches on it instead of reading prose.

use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::header::AUTHORIZATION;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Error;
use crate::http::TaskSummary;
use crate::product;
use crate::state::AppState;
use crate::task::{
    self, Check, NewTask, ReportOutcome, ReviewVerdict, Task, TaskKind, TaskPatch, TaskStatus,
};

/// What a client shows in its server list. `Implementation::from_build_env`
/// reads the SDK's own manifest, so it would answer `rmcp` instead of us.
fn identity() -> Implementation {
    Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskCreateArgs {
    /// Stable identifier, unique and shaped as one URL path segment.
    pub id: String,
    /// One line saying what the work is.
    pub title: String,
    /// Markdown detail. Optional.
    #[serde(default)]
    pub body: Option<String>,
    /// The product this work belongs to, as `org/repo`. It does not have to be
    /// in the catalogue yet; only the promotion to `ready` needs that.
    #[serde(default)]
    pub product_id: Option<String>,
    /// Higher is handed out first. Defaults to 0.
    #[serde(default)]
    pub priority: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskIdArgs {
    /// The task to read.
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskListArgs {
    /// One of `draft`, `ready`, `wip`, `done`, `approved`, `merged`,
    /// `released`, `blocked`, `cancelled`, `dropped`. Left out, everything that
    /// is not `released`.
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskUpdateArgs {
    /// The task to change.
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub product_id: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    /// The git branch the work lives on.
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskSetStatusArgs {
    /// The task to move.
    pub id: String,
    /// The status to move it to.
    pub status: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskClaimArgs {
    /// Who is taking the work, so an abandoned lease can be traced.
    pub worker: String,
    /// The kinds of work this loop handles: `normal`, `instant:merge`,
    /// `review`. Left out, anything claimable.
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
    /// A fresh key for one logical claim attempt. Retrying it recovers the same
    /// live lease when the first response was lost.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskReportArgs {
    /// The `claim_id` the claim handed back.
    pub claim_id: String,
    /// The commit the work landed on.
    pub commit_sha: String,
    /// What was run to believe the work is done.
    pub verification: String,
    /// One entry per verification, with the process exit code. A merge task is
    /// only accepted when every one of them exited 0.
    #[serde(default)]
    pub checks: Vec<CheckArgs>,
    /// `done` (the default) or `blocked`: the work could not be finished, and
    /// `verification` is the reason. A blocked report is kept, not refused.
    #[serde(default)]
    pub outcome: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskReviewReportArgs {
    /// The `claim_id` the claim handed back.
    pub claim_id: String,
    /// The commit that was read. It has to be the one the review was issued for.
    pub subject_commit_sha: String,
    /// `approve` or `request_changes`.
    pub verdict: String,
    /// What the review found. Kept whichever way the verdict went.
    pub findings: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CheckArgs {
    /// The command that was run.
    pub name: String,
    /// Its process exit code; `0` is the only pass.
    pub exit_code: i64,
}

impl From<CheckArgs> for Check {
    fn from(check: CheckArgs) -> Self {
        Self {
            name: check.name,
            exit_code: check.exit_code,
        }
    }
}

/// Encode a domain answer. Success carries the value; a refusal carries the
/// same `{"error", "code"}` pair the HTTP API answers with.
fn answer(result: Result<Value, Error>) -> CallToolResult {
    match result {
        Ok(value) => CallToolResult::structured(value),
        Err(error) => CallToolResult::structured_error(json!({
            "error": error.to_string(),
            "code": error.code(),
        })),
    }
}

/// One task with the statuses it may move to next and the verdict of its latest
/// finished review, the same card the HTTP API answers with.
fn card(db: &crate::db::Db, task: &Task) -> Result<Value, Error> {
    let available_transitions = task::available_transitions(task);
    let latest_review = if task.kind == TaskKind::Normal {
        task::latest_review(db, &task.id)?
    } else {
        None
    };
    Ok(json!({
        "task": task,
        "available_transitions": available_transitions,
        "latest_review": latest_review,
    }))
}

/// The administrative tools: the catalogue and the task lifecycle.
#[derive(Clone)]
pub struct Admin {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

impl Admin {
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl Admin {
    #[tool(
        description = "List the product catalogue. A task is only promoted to `ready` when its \
                       product_id is one of these and that product is not archived, so read this \
                       before filing work. `archived: true` means the working copy left the \
                       project tree: the product still answers for the tasks that named it, but \
                       new work cannot be promoted against it."
    )]
    fn product_list(&self) -> CallToolResult {
        answer(product::list(&self.state.db).map(|products| json!({ "products": products })))
    }

    #[tool(
        description = "Register a new task. It starts in `draft`. Registration is never gated: \
                       `product_id` may name a product that is not in the catalogue yet, and the \
                       refusal arrives later, when the task is promoted to `ready`. This files \
                       ordinary work; merge and review tasks belong to the control plane, which \
                       issues them over HTTP (POST /api/merges, POST /api/reviews), and are not \
                       filed here."
    )]
    fn task_create(&self, Parameters(args): Parameters<TaskCreateArgs>) -> CallToolResult {
        let now = self.state.clock.now();
        answer(
            task::create(
                &self.state.db,
                &NewTask {
                    id: args.id,
                    title: args.title,
                    body: args.body.unwrap_or_default(),
                    product_id: args.product_id,
                    // Not an argument: `instant:merge` belongs to the control
                    // plane, so the model is never offered the choice.
                    kind: TaskKind::Normal,
                    priority: args.priority.unwrap_or(0),
                },
                now,
            )
            .and_then(|task| card(&self.state.db, &task)),
        )
    }

    #[tool(description = "Read one task with the statuses it may move to next.")]
    fn task_get(&self, Parameters(args): Parameters<TaskIdArgs>) -> CallToolResult {
        answer(task::get(&self.state.db, &args.id).and_then(|task| card(&self.state.db, &task)))
    }

    #[tool(
        description = "List tasks. With `status`, only that status; without it, everything that \
                       is not `released` yet."
    )]
    fn task_list(&self, Parameters(args): Parameters<TaskListArgs>) -> CallToolResult {
        let tasks = match args.status {
            Some(raw) => TaskStatus::parse(&raw)
                .and_then(|status| task::list_by_status(&self.state.db, status)),
            None => task::list_active(&self.state.db),
        };
        answer(tasks.map(|tasks| {
            let tasks: Vec<TaskSummary> = tasks.into_iter().map(TaskSummary::from).collect();
            json!({ "tasks": tasks })
        }))
    }

    #[tool(
        description = "Change a task's attributes. Anything left out keeps its current value. \
                       Status is not an attribute; move it with task_set_status."
    )]
    fn task_update(&self, Parameters(args): Parameters<TaskUpdateArgs>) -> CallToolResult {
        let patch = TaskPatch {
            title: args.title,
            body: args.body,
            product_id: args.product_id,
            priority: args.priority,
            branch: args.branch,
        };
        answer(
            task::update(&self.state.db, &args.id, &patch, self.state.clock.now())
                .and_then(|task| card(&self.state.db, &task)),
        )
    }

    #[tool(
        description = "Move a task to another status. A task cannot be promoted to `ready` while \
                       its product is not in the product catalogue: that refusal comes back with \
                       code `product_not_catalogued` (or `product_required` when the task names \
                       no product at all). Correct the id or register the product through the \
                       configured catalogue source: the project tree when APP_PROJECTS_DIR is set, \
                       otherwise PUT /api/products/{id}. Code `product_archived` is different: the \
                       derived catalogue remembers a clone that left its tree, so restore it. \
                       `approved`, `merged` and `released` are granted by the review, merge and \
                       release control plane and are refused here. `done` on a review task is \
                       refused too: a review is finished by its verdict \
                       (POST /worker/review-report), never by a status change."
    )]
    fn task_set_status(&self, Parameters(args): Parameters<TaskSetStatusArgs>) -> CallToolResult {
        let now = self.state.clock.now();
        answer(
            TaskStatus::parse(&args.status)
                .and_then(|to| task::set_status_by_operator(&self.state.db, &args.id, to, now))
                .and_then(|task| card(&self.state.db, &task)),
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Admin {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(identity())
            .with_instructions(
            "Task control plane. File work with task_create, groom it with task_update, and move \
             it with task_set_status. The product catalogue is either curated through the HTTP \
             API or derived from APP_PROJECTS_DIR. Code `product_not_catalogued` means the id is \
             absent from the configured source: correct the product_id, add it through PUT \
             /api/products/{id} in curated mode, or place its clone in the project tree in derived \
             mode. Code `product_archived` means a clone left a derived tree, so restore it.",
        )
    }
}

/// The worker tools: take work, report it finished. Nothing else.
#[derive(Clone)]
pub struct Worker {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

impl Worker {
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl Worker {
    #[tool(
        description = "Claim the next ready task for `worker` and move it to `wip`. Answers \
                       {\"status\":\"no-work\"} when nothing is claimable — that is an answer, \
                       not a failure. A claimed task carries the `claim_id` task_report needs \
                       and the branch the work belongs on. Give each logical attempt a fresh \
                       `idempotency_key`; retrying it recovers the same live lease after an \
                       uncertain response. Pass `kinds` to take only the work this loop handles, \
                       such as [\"review\"] for a reviewer; left out, anything claimable. Merges \
                       of one product run one at a time, but their ready order is not promised."
    )]
    fn task_claim(&self, Parameters(args): Parameters<TaskClaimArgs>) -> CallToolResult {
        let kinds = match args.kinds.as_deref() {
            Some(raw) => raw.iter().map(|kind| TaskKind::parse(kind)).collect(),
            None => Ok(Vec::new()),
        };
        let kinds = match kinds {
            Ok(kinds) => kinds,
            Err(error) => return answer(Err(error)),
        };
        let leased = match args.idempotency_key.as_deref() {
            Some(key) => task::claim_idempotently(
                &self.state.db,
                &args.worker,
                &kinds,
                key,
                self.state.clock.now(),
                self.state.claim_ttl_secs,
            ),
            None => task::claim(
                &self.state.db,
                &args.worker,
                &kinds,
                self.state.clock.now(),
                self.state.claim_ttl_secs,
            ),
        };
        answer(leased.map(|leased| match leased {
            Some(task) => json!({ "status": "claimed", "task": task }),
            None => json!({ "status": "no-work" }),
        }))
    }

    #[tool(
        description = "Report the finished work of a claim, moving the task to `done`. Reporting \
                       the same commit twice is accepted. A merge task is only accepted when \
                       every check exited 0. Finishing ordinary work issues the review that \
                       reads it, in the same transaction. Pass outcome=\"blocked\" instead when \
                       the work could not be finished — a rebase that conflicted, a check that \
                       failed — and `verification` becomes the reason: the task is moved to \
                       `blocked` with that reason and those checks kept, and whatever a merge \
                       would have landed stays where it is."
    )]
    fn task_report(&self, Parameters(args): Parameters<TaskReportArgs>) -> CallToolResult {
        let checks: Vec<Check> = args.checks.into_iter().map(Check::from).collect();
        let outcome = ReportOutcome::parse_optional(args.outcome.as_deref());
        answer(
            outcome
                .and_then(|outcome| {
                    task::report(
                        &self.state.db,
                        &args.claim_id,
                        &args.commit_sha,
                        &args.verification,
                        &checks,
                        outcome,
                        self.state.clock.now(),
                    )
                })
                .and_then(|task| card(&self.state.db, &task)),
        )
    }

    #[tool(
        description = "Answer a claimed review task with a verdict, finishing it. `verdict` is \
                       `approve` or `request_changes`; both are successes, because a review \
                       that asks for changes did its job. `subject_commit_sha` has to be the \
                       commit the review was issued for. An approval moves the reviewed task to \
                       `approved` in the same transaction, and is refused with code \
                       `review_subject_changed` when that task has moved on to another commit, \
                       `review_target_moved` when it is no longer waiting in `done`, and \
                       `review_subject_mismatch` when the commit named is not the one under \
                       review. An approval also issues the merge that lands the work, in that \
                       same transaction, so nothing waits for a human afterwards. \
                       `request_changes` hands the task back to `ready` with the findings on \
                       the record."
    )]
    fn task_review_report(
        &self,
        Parameters(args): Parameters<TaskReviewReportArgs>,
    ) -> CallToolResult {
        answer(
            ReviewVerdict::parse(&args.verdict)
                .and_then(|verdict| {
                    task::review_report(
                        &self.state.db,
                        &args.claim_id,
                        &args.subject_commit_sha,
                        verdict,
                        &args.findings,
                        self.state.clock.now(),
                    )
                })
                .and_then(|task| card(&self.state.db, &task)),
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Worker {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(identity())
            .with_instructions(
                "Worker face of the task control plane. Claim a task, deliver it on the branch the \
             claim names, then report the commit and what was run to verify it.",
            )
    }
}

/// Both MCP endpoints, each behind its own bearer capability.
pub fn endpoints<S>(state: &AppState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    endpoint(
        "/mcp",
        Admin::new(state.clone()),
        state.mcp_capability.clone(),
    )
    .merge(endpoint(
        "/worker/mcp",
        Worker::new(state.clone()),
        state.worker_capability.clone(),
    ))
}

/// One Streamable HTTP endpoint, refusing anything that does not present
/// `capability` before the MCP service ever sees the request.
///
/// rmcp answers loopback `Host` values alone unless told otherwise, as a guard
/// against a page that re-resolves its own name to `127.0.0.1`. That guard is
/// off here: this server is reached through a reverse proxy that already
/// decides which names it serves, and the default would refuse the proxy's own
/// name. The bearer capability remains the gate on the endpoint itself.
fn endpoint<S, H>(path: &str, handler: H, capability: String) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    H: ServerHandler + Clone + Send + Sync + 'static,
{
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().disable_allowed_hosts(),
    );
    Router::new()
        .nest_service(path, service)
        .route_layer(middleware::from_fn(move |request: Request, next: Next| {
            let capability = capability.clone();
            async move { authorize(&capability, request, next).await }
        }))
}

/// `Authorization: Bearer <capability>` or nothing at all. The refusal is the
/// server's own shape, never a JSON-RPC body: a client that never got past this
/// has no session and no protocol to answer in.
async fn authorize(capability: &str, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let (scheme, token) = value.split_once(' ')?;
            scheme
                .eq_ignore_ascii_case("bearer")
                .then(|| token.trim_start())
        });
    if capability.is_empty() || presented != Some(capability) {
        return Error::Unauthorized.into_response();
    }
    next.run(request).await
}
