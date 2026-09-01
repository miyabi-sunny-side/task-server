//! Contract tests for the two MCP endpoints.
//!
//! They speak the wire an MCP client speaks: a Streamable HTTP `POST` that
//! accepts both `application/json` and `text/event-stream`, an `initialize`
//! handshake whose `Mcp-Session-Id` is carried by every later request, and
//! responses read out of the SSE frames. The router is built once and cloned
//! per request, because the session lives inside the service the router owns.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use time::macros::datetime;
use tower::ServiceExt;

use task_server::db::Db;
use task_server::{AppState, SharedClock};

const USER: &str = "miyabi";
const ORIGIN: &str = "https://task-server.test";
const CSRF: &str = "test-csrf";
const STALE_WORKER_CAPABILITY: &str = "old-test-capability";
const MCP_CAPABILITY: &str = "test-mcp-capability";
const PROTOCOL: &str = "2025-06-18";

fn file_backed_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("tempdir");
    let db = Arc::new(Db::open(dir.path().join("state/task-server.db")).expect("open db"));
    let state = AppState::for_test()
        .with_db(db)
        .with_clock(Arc::new(SharedClock::at(
            datetime!(2026-08-15 10:00:00 UTC),
        )));
    (dir, state)
}

/// One MCP connection: an endpoint, optional bearer authorization, and the
/// session the handshake handed back.
struct McpClient {
    router: Router,
    path: &'static str,
    capability: Option<String>,
    session: Option<String>,
    next_id: i64,
}

impl McpClient {
    fn new(router: &Router, path: &'static str, capability: &str) -> Self {
        Self {
            router: router.clone(),
            path,
            capability: Some(capability.to_owned()),
            session: None,
            next_id: 0,
        }
    }

    fn without_capability(router: &Router, path: &'static str) -> Self {
        Self {
            router: router.clone(),
            path,
            capability: None,
            session: None,
            next_id: 0,
        }
    }

    /// A raw POST, so a test can assert on refusals that never reach JSON-RPC.
    async fn post(&self, capability: Option<&str>, body: Value) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method("POST")
            .uri(self.path)
            .header("host", "localhost")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        if let Some(capability) = capability {
            request = request.header("authorization", format!("Bearer {capability}"));
        }
        if let Some(session) = &self.session {
            request = request
                .header("mcp-session-id", session)
                .header("mcp-protocol-version", PROTOCOL);
        }
        let response = self
            .router
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).expect("request"))
            .await
            .expect("router response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (
            status,
            String::from_utf8(bytes.to_vec()).expect("utf-8 body"),
        )
    }

    /// Send one JSON-RPC request and return its `result`, failing on any error.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut request = Request::builder()
            .method("POST")
            .uri(self.path)
            .header("host", "localhost")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        if let Some(capability) = &self.capability {
            request = request.header("authorization", format!("Bearer {capability}"));
        }
        if let Some(session) = &self.session {
            request = request
                .header("mcp-session-id", session)
                .header("mcp-protocol-version", PROTOCOL);
        }
        let response = self
            .router
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).expect("request"))
            .await
            .expect("router response");
        let status = response.status();
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            self.session = Some(session.to_owned());
        }
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let text = String::from_utf8(bytes.to_vec()).expect("utf-8 body");
        assert!(
            status.is_success(),
            "{method} on {} answered {status}: {text}",
            self.path
        );
        let message = find_response(&text, id)
            .unwrap_or_else(|| panic!("no JSON-RPC response for {method} in: {text}"));
        assert!(
            message.get("error").is_none(),
            "{method} failed at the protocol level: {message}"
        );
        message
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("no result for {method} in: {text}"))
    }

    /// A notification carries no id and is only acknowledged.
    async fn notify(&self, method: &str) {
        let (status, text) = self
            .post(
                self.capability.as_deref(),
                json!({ "jsonrpc": "2.0", "method": method }),
            )
            .await;
        assert!(status.is_success(), "{method} answered {status}: {text}");
    }

    async fn initialize(&mut self) -> Value {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL,
                    "capabilities": {},
                    "clientInfo": { "name": "contract-test", "version": "1" },
                }),
            )
            .await;
        assert!(
            self.session.is_some(),
            "the handshake must hand back an Mcp-Session-Id"
        );
        self.notify("notifications/initialized").await;
        result
    }

    async fn tool_names(&mut self) -> Vec<String> {
        let result = self.request("tools/list", json!({})).await;
        let mut names: Vec<String> = result["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_owned())
            .collect();
        names.sort();
        names
    }

    /// Every tool a client can read has to say what it is for.
    async fn tool_descriptions(&mut self) -> Vec<(String, String)> {
        let result = self.request("tools/list", json!({})).await;
        result["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| {
                (
                    tool["name"].as_str().expect("tool name").to_owned(),
                    tool["description"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }

    async fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    /// The declared argument schema of one tool, as a client reads it.
    async fn tool_schema(&mut self, wanted: &str) -> Value {
        let result = self.request("tools/list", json!({})).await;
        result["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .find(|tool| tool["name"] == wanted)
            .unwrap_or_else(|| panic!("no tool named {wanted} in {result}"))
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| panic!("tool {wanted} declares no inputSchema"))
    }
}

fn handshake() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": { "name": "contract-test", "version": "1" },
        },
    })
}

/// A raw POST that chooses its own `Host`, the header rmcp would have refused
/// on while its allowlist was still in play.
async fn post_with_host(
    router: &Router,
    path: &str,
    capability: &str,
    host: &str,
    body: &Value,
) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", host)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {capability}"))
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(bytes.as_ref()).into_owned())
}

/// Pull the JSON-RPC message with `id` out of an SSE body (or a plain JSON one).
fn find_response(body: &str, id: i64) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(body)
        && value.get("id").and_then(Value::as_i64) == Some(id)
    {
        return Some(value);
    }
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .find(|value| value.get("id").and_then(Value::as_i64) == Some(id))
}

/// A human mutation over the existing HTTP surface, used to prove that MCP and
/// HTTP share one database.
async fn http(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-auth-user", USER)
        .header("origin", ORIGIN)
        .header("x-csrf-token", CSRF);
    let body = match body {
        Some(value) => {
            request = request.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("router response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn catalogue(router: &Router, id: &str) {
    let (status, _) = http(
        router,
        "PUT",
        &format!("/api/products/{id}"),
        Some(json!({ "repository": format!("https://example.test/{id}.git") })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "cataloguing {id}");
}

#[tokio::test]
async fn both_endpoints_handshake_and_expose_only_their_own_tools() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);

    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    let info = admin.initialize().await;
    assert!(
        info["capabilities"].get("tools").is_some(),
        "the admin endpoint must advertise tools: {info}"
    );
    assert_eq!(
        info["serverInfo"]["name"], "task-server",
        "a client lists us by name, not by the SDK we happen to use: {info}"
    );
    assert_eq!(
        info["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION"),
        "the advertised version is this crate's: {info}"
    );
    assert_eq!(
        admin.tool_names().await,
        [
            "product_list",
            "task_create",
            "task_get",
            "task_list",
            "task_set_status",
            "task_update",
        ],
        "the admin endpoint owns the catalogue-and-task surface"
    );

    let mut worker = McpClient::without_capability(&router, "/worker/mcp");
    worker.initialize().await;
    assert_eq!(
        worker.tool_names().await,
        ["task_claim", "task_report", "task_review_report"],
        "the open worker endpoint never exposes task CRUD"
    );

    for (name, description) in admin.tool_descriptions().await {
        assert!(
            !description.trim().is_empty(),
            "tool {name} has no description for the model to read"
        );
        if name == "task_set_status" {
            assert!(
                description.contains("catalogue") || description.contains("catalogued"),
                "task_set_status must warn about the catalogue gate: {description}"
            );
        }
    }

    // Bearer refusals never reach the JSON-RPC layer.
    let handshake = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": { "name": "contract-test", "version": "1" },
        },
    });
    let anonymous = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    // A handshake is answered in SSE frames, which is what makes this endpoint
    // Streamable HTTP rather than a plain JSON-RPC POST.
    let (status, framed) = anonymous
        .post(Some(MCP_CAPABILITY), handshake.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{framed}");
    assert!(
        framed.contains("data:"),
        "the endpoint answers SSE frames: {framed}"
    );

    for capability in [None, Some("wrong"), Some(STALE_WORKER_CAPABILITY)] {
        let (status, body) = anonymous.post(capability, handshake.clone()).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "/mcp with {capability:?} must be refused: {body}"
        );
        assert!(
            !body.contains("jsonrpc"),
            "a refused request gets no JSON-RPC answer: {body}"
        );
    }

    let anonymous_worker = McpClient::without_capability(&router, "/worker/mcp");
    for capability in [
        None,
        Some("wrong"),
        Some(MCP_CAPABILITY),
        Some(STALE_WORKER_CAPABILITY),
    ] {
        let (status, body) = anonymous_worker.post(capability, handshake.clone()).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "/worker/mcp ignores obsolete authorization during rollout: {body}"
        );
        assert!(
            body.contains("data:"),
            "accepted MCP is framed as SSE: {body}"
        );
    }
}

#[tokio::test]
async fn mcp_registration_lands_but_ready_is_refused_until_the_product_is_catalogued() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);

    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;

    let created = admin
        .call(
            "task_create",
            json!({
                "id": "t-1",
                "title": "teach the gate",
                "body": "filed through MCP",
                "product_id": "nobody/knows",
            }),
        )
        .await;
    assert_eq!(
        created["isError"],
        json!(false),
        "registration is not gated: {created}"
    );
    assert_eq!(created["structuredContent"]["task"]["status"], "draft");

    let refused = admin
        .call("task_set_status", json!({ "id": "t-1", "status": "ready" }))
        .await;
    assert_eq!(
        refused["isError"],
        json!(true),
        "an uncatalogued promotion is a tool error: {refused}"
    );
    assert_eq!(
        refused["structuredContent"]["code"], "product_not_catalogued",
        "the model branches on the code: {refused}"
    );
    let message = refused["structuredContent"]["error"]
        .as_str()
        .expect("error message");
    assert!(
        message.contains("nobody/knows"),
        "the refusal names the product: {message}"
    );
    let text = refused["content"][0]["text"]
        .as_str()
        .expect("text content");
    let echoed: Value = serde_json::from_str(text).expect("text content is the same JSON");
    assert_eq!(echoed, refused["structuredContent"]);

    let unchanged = admin.call("task_get", json!({ "id": "t-1" })).await;
    assert_eq!(
        unchanged["structuredContent"]["task"]["status"], "draft",
        "a refused promotion leaves the row where it was: {unchanged}"
    );

    // The remedy is the catalogue, and it is a human decision over HTTP.
    catalogue(&router, "nobody/knows").await;

    let promoted = admin
        .call("task_set_status", json!({ "id": "t-1", "status": "ready" }))
        .await;
    assert_eq!(promoted["isError"], json!(false), "{promoted}");
    assert_eq!(promoted["structuredContent"]["task"]["status"], "ready");

    let listed = admin.call("product_list", json!({})).await;
    let ids: Vec<&str> = listed["structuredContent"]["products"]
        .as_array()
        .expect("products")
        .iter()
        .map(|product| product["id"].as_str().expect("product id"))
        .collect();
    assert!(ids.contains(&"nobody/knows"), "{listed}");
}

#[tokio::test]
async fn a_worker_drives_a_task_over_mcp_and_http_sees_the_same_row() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);
    catalogue(&router, "sunny-side/task-server").await;

    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;
    admin
        .call(
            "task_create",
            json!({
                "id": "t-1",
                "title": "drive it over mcp",
                "product_id": "sunny-side/task-server",
                "priority": 5,
            }),
        )
        .await;
    let ready = admin
        .call("task_set_status", json!({ "id": "t-1", "status": "ready" }))
        .await;
    assert_eq!(ready["structuredContent"]["task"]["status"], "ready");

    let mut worker = McpClient::new(&router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;

    let claimed = worker
        .call("task_claim", json!({ "worker": "loop-1" }))
        .await;
    assert_eq!(claimed["isError"], json!(false), "{claimed}");
    let task = &claimed["structuredContent"]["task"];
    assert_eq!(task["id"], "t-1");
    assert_eq!(task["status"], "wip");
    assert_eq!(task["branch"], "task/t-1");
    let claim_id = task["claim_id"].as_str().expect("claim id").to_owned();

    let empty = worker
        .call("task_claim", json!({ "worker": "loop-2" }))
        .await;
    assert_eq!(
        empty["structuredContent"]["status"], "no-work",
        "an idle queue is an answer, not a failure: {empty}"
    );
    assert_eq!(empty["isError"], json!(false), "{empty}");

    let reported = worker
        .call(
            "task_report",
            json!({
                "claim_id": claim_id,
                "commit_sha": "abc1234",
                "verification": "cargo test",
                "checks": [{ "name": "cargo test", "exit_code": 0 }],
            }),
        )
        .await;
    assert_eq!(reported["isError"], json!(false), "{reported}");
    assert_eq!(reported["structuredContent"]["task"]["status"], "done");

    // The proof that both transports write one sqlite: HTTP reads the same row.
    let (status, card) = http(&router, "GET", "/api/tasks/t-1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(card["status"], "done");
    assert_eq!(card["branch"], "task/t-1");
    assert_eq!(card["commit_sha"], "abc1234");
    assert_eq!(card["verification"], "cargo test");
    assert_eq!(card["checks"][0]["name"], "cargo test");

    let seen = admin.call("task_get", json!({ "id": "t-1" })).await;
    assert_eq!(seen["structuredContent"]["task"]["commit_sha"], "abc1234");

    let listed = admin.call("task_list", json!({ "status": "done" })).await;
    let ids: Vec<&str> = listed["structuredContent"]["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .map(|task| task["id"].as_str().expect("task id"))
        .collect();
    assert_eq!(ids, ["t-1"]);

    let updated = admin
        .call(
            "task_update",
            json!({ "id": "t-1", "title": "renamed over mcp" }),
        )
        .await;
    assert_eq!(
        updated["structuredContent"]["task"]["title"],
        "renamed over mcp"
    );
    let (_, card) = http(&router, "GET", "/api/tasks/t-1", None).await;
    assert_eq!(card["title"], "renamed over mcp");
}

/// The `Host` a client sends decides nothing here. A deployment answers to its
/// own name behind a reverse proxy, and that proxy is what chooses the names it
/// serves — leaving rmcp's loopback default on would only refuse the name the
/// proxy forwards. The bearer capability stays the gate on the endpoint.
#[tokio::test]
async fn any_host_reaches_both_endpoints() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);

    for path in ["/mcp", "/worker/mcp"] {
        let capability = if path == "/mcp" {
            MCP_CAPABILITY
        } else {
            STALE_WORKER_CAPABILITY
        };
        for host in [
            "tasks.example.test",
            "evil.test",
            "localhost",
            "127.0.0.1:3000",
        ] {
            let (status, body) =
                post_with_host(&router, path, capability, host, &handshake()).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "{path} must answer a client sending Host {host}: {body}"
            );
        }
    }
}

/// `instant:merge` is issued by the control plane and by nothing else, so the
/// model is never even offered the choice: `task_create` has no `kind` at all.
#[tokio::test]
async fn task_create_has_no_kind_and_cannot_file_a_merge() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);

    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;

    let schema = admin.tool_schema("task_create").await;
    let properties = schema["properties"]
        .as_object()
        .expect("task_create declares its properties");
    assert!(
        properties.contains_key("id") && properties.contains_key("title"),
        "the schema must still describe the task: {schema}"
    );
    assert!(
        !properties.contains_key("kind"),
        "task_create must not offer kind: {schema}"
    );

    // And the wire is closed too: a client that sends `kind` anyway files
    // ordinary work, never an instant merge.
    let created = admin
        .call(
            "task_create",
            json!({
                "id": "t-1",
                "title": "smuggle a merge",
                "product_id": "sunny-side/task-server",
                "kind": "instant:merge",
            }),
        )
        .await;
    assert_eq!(
        created["structuredContent"]["task"]["kind"], "normal",
        "a smuggled kind must not produce an instant merge: {created}"
    );

    let stored = admin.call("task_get", json!({ "id": "t-1" })).await;
    assert_eq!(
        stored["structuredContent"]["task"]["kind"], "normal",
        "no instant merge may exist in the store: {stored}"
    );

    let listed = admin.call("task_list", json!({})).await;
    for task in listed["structuredContent"]["tasks"]
        .as_array()
        .expect("tasks")
    {
        assert_eq!(
            task["kind"], "normal",
            "MCP may not put an instant merge in the queue: {task}"
        );
    }
}

/// The refusal of `approved`, `merged`, and `released` is one domain rule, so
/// pressing it over MCP answers exactly what pressing it over HTTP answers, and
/// moves nothing. `approved` matters most here: a model that could press it
/// would be approving its own work.
#[tokio::test]
async fn mcp_status_change_refuses_approved_merged_and_released() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);
    catalogue(&router, "sunny-side/task-server").await;

    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;
    admin
        .call(
            "task_create",
            json!({
                "id": "t-1",
                "title": "land me the wrong way",
                "product_id": "sunny-side/task-server",
            }),
        )
        .await;
    admin
        .call("task_set_status", json!({ "id": "t-1", "status": "ready" }))
        .await;

    let mut worker = McpClient::new(&router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;
    let claimed = worker
        .call("task_claim", json!({ "worker": "loop-1" }))
        .await;
    let claim_id = claimed["structuredContent"]["task"]["claim_id"]
        .as_str()
        .expect("claim id")
        .to_owned();
    let reported = worker
        .call(
            "task_report",
            json!({
                "claim_id": claim_id,
                "commit_sha": "abc1234",
                "verification": "cargo test",
            }),
        )
        .await;
    assert_eq!(reported["structuredContent"]["task"]["status"], "done");

    // `done` is exactly the status the transition table would let through, so
    // this is the bypass the shared rule has to close.
    for status in ["approved", "merged", "released"] {
        let refused = admin
            .call("task_set_status", json!({ "id": "t-1", "status": status }))
            .await;
        assert_eq!(
            refused["isError"],
            json!(true),
            "{status} must be refused over MCP: {refused}"
        );
        assert_eq!(
            refused["structuredContent"]["code"], "invalid",
            "the same stable code HTTP answers with: {refused}"
        );
        let message = refused["structuredContent"]["error"]
            .as_str()
            .expect("error message");
        assert!(
            message.contains("control plane"),
            "the refusal must name the control plane: {message}"
        );

        let (status_code, card) = http(&router, "GET", "/api/tasks/t-1", None).await;
        assert_eq!(status_code, StatusCode::OK);
        assert_eq!(card["status"], "done", "a refusal moves no row: {card}");
    }
}

/// MCP exposes the same retry key and the same lease replay as HTTP; it is a
/// second transport over one claim contract, not a weaker path around it.
#[tokio::test]
async fn an_mcp_claim_retries_the_same_live_lease() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);
    catalogue(&router, "sunny-side/task-server").await;

    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;
    admin
        .call(
            "task_create",
            json!({
                "id": "t-idempotent",
                "title": "retry me",
                "product_id": "sunny-side/task-server",
            }),
        )
        .await;
    admin
        .call(
            "task_set_status",
            json!({ "id": "t-idempotent", "status": "ready" }),
        )
        .await;

    let mut worker = McpClient::new(&router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;
    let schema = worker.tool_schema("task_claim").await;
    assert!(
        schema["properties"].get("idempotency_key").is_some(),
        "task_claim must declare its retry key: {schema}"
    );

    let arguments = json!({
        "worker": "opus",
        "kinds": ["normal"],
        "idempotency_key": "mcp-claim-attempt-1",
    });
    let first = worker.call("task_claim", arguments.clone()).await;
    let replayed = worker.call("task_claim", arguments).await;
    assert_eq!(
        first["structuredContent"]["task"]["id"], "t-idempotent",
        "{first}"
    );
    assert_eq!(
        first["structuredContent"]["task"]["claim_id"],
        replayed["structuredContent"]["task"]["claim_id"],
        "{replayed}"
    );

    let reused = worker
        .call(
            "task_claim",
            json!({
                "worker": "another-worker",
                "kinds": ["normal"],
                "idempotency_key": "mcp-claim-attempt-1",
            }),
        )
        .await;
    assert_eq!(reused["isError"], json!(true), "{reused}");
    assert_eq!(
        reused["structuredContent"]["code"], "claim_idempotency_conflict",
        "{reused}"
    );
}

/// File `id` for the catalogued product and take it to `done` over MCP, the way
/// an implementing loop does.
async fn work_to_done_over_mcp(router: &Router, id: &str) {
    let mut admin = McpClient::new(router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;
    admin
        .call(
            "task_create",
            json!({
                "id": id,
                "title": "read me",
                "product_id": "sunny-side/task-server",
            }),
        )
        .await;
    admin
        .call("task_set_status", json!({ "id": id, "status": "ready" }))
        .await;

    let mut worker = McpClient::new(router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;
    let claimed = worker
        .call(
            "task_claim",
            json!({ "worker": "opus", "kinds": ["normal"] }),
        )
        .await;
    let claim_id = claimed["structuredContent"]["task"]["claim_id"]
        .as_str()
        .expect("claim id")
        .to_owned();
    let reported = worker
        .call(
            "task_report",
            json!({
                "claim_id": claim_id,
                "commit_sha": "abc1234",
                "verification": "cargo test",
            }),
        )
        .await;
    assert_eq!(reported["structuredContent"]["task"]["status"], "done");
}

/// The worker face carries the review contract too, or a reviewer loop on MCP
/// would have to fall back to HTTP for the one call it exists to make.
#[tokio::test]
async fn a_reviewer_answers_over_mcp_and_http_sees_the_same_row() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);
    catalogue(&router, "sunny-side/task-server").await;

    work_to_done_over_mcp(&router, "t-1").await;
    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;
    let mut worker = McpClient::new(&router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;

    // The report of t-1 issued the review; nobody files it by hand.
    let (status, review) = http(&router, "GET", "/api/tasks/review:t-1", None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the report issues a review: {review}"
    );

    // A loop that asks for merges is told there is no work, not handed the
    // review that is waiting.
    let idle = worker
        .call(
            "task_claim",
            json!({ "worker": "luna", "kinds": ["instant:merge"] }),
        )
        .await;
    assert_eq!(idle["structuredContent"]["status"], "no-work", "{idle}");
    assert_eq!(idle["isError"], json!(false), "{idle}");

    let claimed = worker
        .call(
            "task_claim",
            json!({ "worker": "sol", "kinds": ["review"] }),
        )
        .await;
    let review_task = &claimed["structuredContent"]["task"];
    assert_eq!(review_task["id"], "review:t-1");
    assert_eq!(review_task["kind"], "review");
    assert_eq!(review_task["commit_sha"], "abc1234");
    let claim_id = review_task["claim_id"]
        .as_str()
        .expect("claim id")
        .to_owned();

    // An approval of a commit the review was not issued for is refused with the
    // same stable code HTTP answers with, and writes nothing.
    let refused = worker
        .call(
            "task_review_report",
            json!({
                "claim_id": claim_id,
                "subject_commit_sha": "def5678",
                "verdict": "approve",
                "findings": "approving something else",
            }),
        )
        .await;
    assert_eq!(refused["isError"], json!(true), "{refused}");
    assert_eq!(
        refused["structuredContent"]["code"], "review_subject_mismatch",
        "{refused}"
    );

    let answered = worker
        .call(
            "task_review_report",
            json!({
                "claim_id": claim_id,
                "subject_commit_sha": "abc1234",
                "verdict": "request_changes",
                "findings": "the empty case is unguarded",
            }),
        )
        .await;
    assert_eq!(
        answered["isError"],
        json!(false),
        "asking for changes is a finished review, not a failure: {answered}"
    );
    assert_eq!(answered["structuredContent"]["task"]["status"], "done");
    assert_eq!(
        answered["structuredContent"]["task"]["review_verdict"],
        "request_changes"
    );

    // One database: HTTP reads the same row, and the worker reads the findings
    // off the task it will claim again.
    let (status, card) = http(&router, "GET", "/api/tasks/t-1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(card["status"], "ready", "the work went back: {card}");
    assert_eq!(card["latest_review"]["verdict"], "request_changes");
    assert_eq!(
        card["latest_review"]["findings"],
        "the empty case is unguarded"
    );

    let seen = admin.call("task_get", json!({ "id": "t-1" })).await;
    assert_eq!(
        seen["structuredContent"]["latest_review"]["findings"], "the empty case is unguarded",
        "the same card over both transports: {seen}"
    );
}

/// The MCP status tool inherits the merge release contract too: a blocked merge
/// is called off and reissued, never restarted.
///
/// A model driving the board is exactly who would reach for `ready` to get a
/// jammed train moving, so the refusal has to reach this surface with the same
/// stable code HTTP answers with — and `dropped` has to keep working, because
/// calling the attempt off is the way out.
/// Answer the review `t-1`'s report issued with an approval, which is what
/// issues `merge:t-1`. `t-1` must already be `done`.
async fn merge_issued_over_mcp(router: &Router) {
    let mut worker = McpClient::new(router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;
    let claimed = worker
        .call(
            "task_claim",
            json!({ "worker": "sol", "kinds": ["review"] }),
        )
        .await;
    let claim_id = claimed["structuredContent"]["task"]["claim_id"]
        .as_str()
        .expect("claim id")
        .to_owned();
    worker
        .call(
            "task_review_report",
            json!({
                "claim_id": claim_id,
                "subject_commit_sha": "abc1234",
                "verdict": "approve",
                "findings": "reads well",
            }),
        )
        .await;
}

/// Claim `merge:t-1` the way a merge worker does, and hand back the lease it
/// has to report against.
async fn merge_claimed_over_mcp(router: &Router) -> String {
    let mut worker = McpClient::new(router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;
    let claimed = worker
        .call(
            "task_claim",
            json!({ "worker": "luna", "kinds": ["instant:merge"] }),
        )
        .await;
    assert_eq!(
        claimed["structuredContent"]["task"]["id"], "merge:t-1",
        "the approval issues the merge: {claimed}"
    );
    claimed["structuredContent"]["task"]["claim_id"]
        .as_str()
        .expect("claim id")
        .to_owned()
}

/// Take `t-1` from `done` all the way to a merge that reported it could not be
/// integrated: approve the review the report issued, claim the merge that
/// approval issued, and block it.
async fn merge_blocked_over_mcp(router: &Router) {
    merge_issued_over_mcp(router).await;
    let claim_id = merge_claimed_over_mcp(router).await;
    let mut worker = McpClient::new(router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;
    let blocked = worker
        .call(
            "task_report",
            json!({
                "claim_id": claim_id,
                "commit_sha": "abc1234",
                "verification": "rebase onto main conflicts in src/task.rs",
                "checks": [{"name": "git rebase", "exit_code": 1}],
                "outcome": "blocked",
            }),
        )
        .await;
    assert_eq!(blocked["isError"], json!(false), "{blocked}");
    assert_eq!(
        blocked["structuredContent"]["task"]["status"], "blocked",
        "{blocked}"
    );
}

/// Press both outcomes on `merge:t-1` over MCP and insist on the refusal.
async fn assert_merge_outcomes_are_refused_over_mcp(admin: &mut McpClient) {
    for pressed in ["done", "blocked"] {
        let refused = admin
            .call(
                "task_set_status",
                json!({ "id": "merge:t-1", "status": pressed }),
            )
            .await;
        assert_eq!(
            refused["isError"],
            json!(true),
            "a press must not write the outcome of a merge: {refused}"
        );
        assert_eq!(
            refused["structuredContent"]["code"], "invalid",
            "the same stable code HTTP answers with: {refused}"
        );
        let message = refused["structuredContent"]["error"]
            .as_str()
            .expect("error message");
        assert!(
            message.contains("/worker/report"),
            "the refusal must name where the answer comes from: {message}"
        );
    }
}

/// The MCP status tool cannot say how a merge *ended* either.
///
/// A model driving the board is exactly who would press `done` to tidy a queue
/// up or `blocked` to park an attempt, and both are the worker's answer. The
/// refusal has to reach this surface with the same stable code HTTP answers
/// with, and the report has to keep working straight through it.
#[tokio::test]
async fn mcp_status_change_cannot_write_the_outcome_of_a_merge() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);
    catalogue(&router, "sunny-side/task-server").await;

    work_to_done_over_mcp(&router, "t-1").await;
    merge_issued_over_mcp(&router).await;
    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;

    assert_merge_outcomes_are_refused_over_mcp(&mut admin).await;

    let card = admin.call("task_get", json!({ "id": "merge:t-1" })).await;
    let card = &card["structuredContent"];
    assert_eq!(
        card["task"]["status"], "ready",
        "a refusal moves no row: {card}"
    );
    assert_eq!(
        card["task"]["verification"],
        Value::Null,
        "and writes no reason onto it: {card}"
    );
    assert_eq!(
        card["available_transitions"]
            .as_array()
            .expect("available_transitions"),
        &vec![json!("wip"), json!("cancelled"), json!("dropped")],
        "no outcome is offered on a merge: {card}"
    );

    // The two windows a pressed `done` would have emptied together.
    let (status, plane) = http(&router, "GET", "/api/control", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plane["mergeable"], json!([]), "{plane}");
    assert_eq!(plane["pending_merges"][0]["id"], "merge:t-1", "{plane}");

    // Running, and refused the same way; then the worker's own report lands it.
    let claim_id = merge_claimed_over_mcp(&router).await;
    assert_merge_outcomes_are_refused_over_mcp(&mut admin).await;
    let (status, target) = http(&router, "GET", "/api/tasks/t-1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        target["status"], "approved",
        "the target has not moved: {target}"
    );

    let mut worker = McpClient::new(&router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;
    // The same `outcome` contract HTTP holds: a name nobody defined is refused
    // rather than read as the success that would land this merge.
    let typo = worker
        .call(
            "task_report",
            json!({
                "claim_id": claim_id,
                "commit_sha": "abc1234",
                "verification": "merged onto main",
                "checks": [{"name": "cargo test", "exit_code": 0}],
                "outcome": "Done",
            }),
        )
        .await;
    assert_eq!(typo["isError"], json!(true), "{typo}");
    assert_eq!(typo["structuredContent"]["code"], "invalid", "{typo}");
    let (status, target) = http(&router, "GET", "/api/tasks/t-1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        target["status"], "approved",
        "the refused report landed nothing: {target}"
    );

    let landed = worker
        .call(
            "task_report",
            json!({
                "claim_id": claim_id,
                "commit_sha": "abc1234",
                "verification": "merged onto main",
                "checks": [{"name": "cargo test", "exit_code": 0}],
            }),
        )
        .await;
    assert_eq!(landed["isError"], json!(false), "{landed}");
    assert_eq!(
        landed["structuredContent"]["task"]["status"], "done",
        "an omitted outcome is still `done`: {landed}"
    );
    let (status, target) = http(&router, "GET", "/api/tasks/t-1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        target["status"], "merged",
        "and the report is what moves the target: {target}"
    );
}

#[tokio::test]
async fn mcp_status_change_cannot_restart_a_blocked_merge() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);
    catalogue(&router, "sunny-side/task-server").await;

    work_to_done_over_mcp(&router, "t-1").await;
    merge_blocked_over_mcp(&router).await;
    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;

    let refused = admin
        .call(
            "task_set_status",
            json!({ "id": "merge:t-1", "status": "ready" }),
        )
        .await;
    assert_eq!(
        refused["isError"],
        json!(true),
        "a press must not restart a blocked merge: {refused}"
    );
    assert_eq!(
        refused["structuredContent"]["code"], "invalid",
        "the same stable code HTTP answers with: {refused}"
    );

    let card = admin.call("task_get", json!({ "id": "merge:t-1" })).await;
    let card = &card["structuredContent"];
    assert_eq!(
        card["task"]["status"], "blocked",
        "a refusal moves no row: {card}"
    );
    let offered = card["available_transitions"]
        .as_array()
        .expect("available_transitions");
    assert!(
        !offered.contains(&json!("ready")),
        "ready is never offered on a blocked merge: {card}"
    );
    assert_eq!(
        offered,
        &vec![json!("cancelled"), json!("dropped")],
        "only the presses that call the attempt off are offered: {card}"
    );

    // Dropping it is a release exactly like cancelling, and the merge the work
    // earns afterwards is a new row.
    let dropped = admin
        .call(
            "task_set_status",
            json!({ "id": "merge:t-1", "status": "dropped" }),
        )
        .await;
    assert_eq!(dropped["isError"], json!(false), "{dropped}");

    let (status, reissued) = http(
        &router,
        "POST",
        "/api/merges",
        Some(json!({ "task_id": "t-1" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the dropped attempt freed the target: {reissued}"
    );
    assert_eq!(reissued["id"], "merge:t-1~2", "{reissued}");
    assert_eq!(reissued["verification"], Value::Null, "{reissued}");
}

/// The MCP status tool is the other operator surface, and it inherits the same
/// domain rule: a review is finished by its verdict, so `done` is refused there
/// too. A model that could press it would be closing its own review with no
/// reading behind it, and freeing the target for the next one on the way out.
#[tokio::test]
async fn mcp_status_change_cannot_finish_a_review() {
    let (_dir, state) = file_backed_state();
    let router = task_server::app(state);
    catalogue(&router, "sunny-side/task-server").await;

    work_to_done_over_mcp(&router, "t-1").await;
    let mut admin = McpClient::new(&router, "/mcp", MCP_CAPABILITY);
    admin.initialize().await;
    let mut worker = McpClient::new(&router, "/worker/mcp", STALE_WORKER_CAPABILITY);
    worker.initialize().await;

    let (status, review) = http(&router, "GET", "/api/tasks/review:t-1", None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the report issues a review: {review}"
    );
    let claimed = worker
        .call(
            "task_claim",
            json!({ "worker": "sol", "kinds": ["review"] }),
        )
        .await;
    assert_eq!(claimed["structuredContent"]["task"]["id"], "review:t-1");

    let refused = admin
        .call(
            "task_set_status",
            json!({ "id": "review:t-1", "status": "done" }),
        )
        .await;
    assert_eq!(
        refused["isError"],
        json!(true),
        "a press must not finish a review: {refused}"
    );
    assert_eq!(
        refused["structuredContent"]["code"], "invalid",
        "the same stable code HTTP answers with: {refused}"
    );
    let message = refused["structuredContent"]["error"]
        .as_str()
        .expect("error message");
    assert!(
        message.contains("verdict"),
        "the refusal must name the verdict: {message}"
    );

    let card = admin.call("task_get", json!({ "id": "review:t-1" })).await;
    let card = &card["structuredContent"];
    assert_eq!(
        card["task"]["status"], "wip",
        "a refusal moves no row: {card}"
    );
    assert_eq!(card["task"]["review_verdict"], Value::Null, "{card}");
    let offered = card["available_transitions"]
        .as_array()
        .expect("available_transitions");
    assert!(
        !offered.contains(&json!("done")),
        "done is never offered on a review: {card}"
    );

    let (status_code, parent) = http(&router, "GET", "/api/tasks/t-1", None).await;
    assert_eq!(status_code, StatusCode::OK);
    assert_eq!(
        parent["status"], "done",
        "the parent is untouched: {parent}"
    );
    assert_eq!(parent["latest_review"], Value::Null, "{parent}");

    let (status_code, again) = http(
        &router,
        "POST",
        "/api/reviews",
        Some(json!({ "task_id": "t-1" })),
    )
    .await;
    assert_eq!(
        status_code,
        StatusCode::CONFLICT,
        "the refused press must not have freed the one-open-review index: {again}"
    );
}
