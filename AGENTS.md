# task-server

Task control plane. One Axum process owns one sqlite database and serves the
JSON API, the MCP endpoints, and the compiled client.

## Invariants

- SQLite is truth. The transaction boundary is a sqlite transaction, taken
  `IMMEDIATE` so concurrent writers serialize at `BEGIN`.
- The database lives at `APP_DB_PATH` (default `data/task-server.db`).
  Migrations run at open and are keyed on `PRAGMA user_version`.
- HTTP is the only way in, carrying both the JSON API and the MCP endpoints.
  There is no file store and no git side effect.
- There is no physical delete. Discarding a task is a transition to
  `cancelled` or `dropped`, so the row stays auditable.
- Product ids are `org/repo`, never a path. Task ids are one path segment.
- The `products` table is the register of product identity. A task may be
  created with a `product_id` that is not in it, but it may not be promoted:
  `ready` is refused with 409 and code `product_not_catalogued` when the product
  is uncatalogued, or `product_required` when the task has no `product_id`. The
  gate fires on the transition only, so a row that is already `ready` or beyond
  is never demoted, and `available_transitions` still offers `ready`.
- `APP_PRODUCTS_SEED` upserts a JSON product roster at startup, in one
  transaction. Unreadable, unparseable, or invalid seeds fail the startup with
  nothing written. The roster itself is operational data and is not in the
  repository.
- Every refusal the domain owns — and every unknown `/api/*` path — answers
  `{"error": "<message>", "code": "<slug>"}`. The slug is stable and is what an
  automated client branches on; the message is not. A request body that is not
  JSON never reaches the domain: the axum extractor rejects it first, with 400
  and a plain-text explanation that carries no `code`.
- `merged` and `released` belong to the control plane. Every operator surface
  goes through `task::set_status_by_operator`, which refuses both before
  delegating to `set_status`, so `POST /api/tasks/{id}/status` and the MCP
  `task_set_status` tool answer the same refusal from one place. The control
  plane keeps calling `set_status` directly, `available_transitions` never
  offers either, and the transition table still allows them because the control
  plane goes through it.
- Only the control plane issues `instant:merge`. `task::create` refuses that
  kind outright, so `POST /api/tasks` answers 400 with code `invalid` and the
  MCP `task_create` tool has no `kind` argument to choose from. A merge task is
  written by `task::issue_merge` alone, against a target it names; an orphan
  merge would be claimed first, could never be reported, and would block the
  queue.
- A task is mergeable when it is `normal`, `done`, carries a `branch` and a
  `commit_sha`, and no live merge already targets it. `POST /api/merges` issues
  one `instant:merge` task per target, in `ready`, inheriting the target's
  `product_id`, `branch`, and `commit_sha`. A partial unique index keeps that at
  one live merge per target; a second issue is 409. A cancelled or dropped
  attempt frees the target for a retry.
- A merge task is `merge:<target_id>`; a retry whose id is already taken appends
  `~2`, `~3`, … so the id stays deterministic and one path segment.
- Nothing lands untested. A report on an `instant:merge` task is refused unless
  it carries `checks` and every `exit_code` is `0`, whatever the merge's current
  status. Accepting it moves the merge to `done` and its target from `done` to
  `merged` in one transaction; a refusal changes neither row.
- A task may only reach `released` when its product has `releases` set.
  `POST /api/releases` moves every `merged` normal task of one product to
  `released` under a single `release_tag`, in one transaction. A product that
  does not release, or one with nothing merged, is 409.
- Claim hands out the next `ready` task, `instant:merge` first, then higher
  `priority`, then oldest. The row is only taken while it is still `ready`,
  so two workers never hold the same task.
- One task, one branch. A claim on a task without a `branch` sets
  `task/<id>`; an existing branch is never rewritten.
- A report is matched by `claim_id`. A stale or unknown `claim_id` is
  rejected with 409. Reporting the same `commit_sha` twice is idempotent.
- Clock is injectable. Default claim TTL is 3600 seconds (`CLAIM_TTL_SECS`).
- Listen on `127.0.0.1` by default (`APP_BIND_ADDR` may override).
- Worker routes require `X-Worker-Capability` equal to the configured secret.
  An identity header alone does not authenticate.
- Human identity comes from ingress (`X-Auth-User` or `Tailscale-User-Login`).
  The browser does not mint identity.
- Human mutation requires an allowlisted ingress identity, a matching
  `Origin`, and `X-CSRF-Token`. Worker capability is not sufficient.
- MCP is a second transport, not a second domain. `/mcp` and `/worker/mcp` are
  Streamable HTTP endpoints in the same process, and every tool decodes its
  arguments and calls `src/task.rs` or `src/product.rs`. The transition table,
  the catalogue gate, and the SQL are never duplicated there.
- Each MCP endpoint has its own bearer: `MCP_CAPABILITY` for `/mcp`,
  `WORKER_CAPABILITY` for `/worker/mcp`. A worker credential never opens task
  CRUD. The check runs before rmcp sees the request, so a missing or mismatched
  bearer is 401 and gets no JSON-RPC answer at all.
- Ingress identity, `Origin`, and CSRF are not applied to MCP; the bearer is the
  gate for authorization. The `Host` allowlist is a separate gate and stays on:
  rmcp's loopback-only default is what stops DNS rebinding, and the bearer does
  not replace it because a development capability is a published constant. An
  undeclared `Host` is refused with 403 before JSON-RPC. `APP_MCP_ALLOWED_HOSTS`
  declares the authorities a published deployment answers to and replaces the
  default; it is never switched off.
- `/mcp` carries `product_list`, `task_create`, `task_get`, `task_list`,
  `task_update`, and `task_set_status`; `/worker/mcp` carries `task_claim` and
  `task_report`. Catalogue writes, merges, and releases are human decisions and
  have no tool; `task_create` files ordinary work and takes no `kind`, and
  `task_set_status` refuses `merged` and `released` with code `invalid` through
  the same domain function the HTTP status route calls.
- A refusal the domain owns is not a protocol failure: it is a tool result with
  `isError: true` whose `structuredContent` is the same `{"error", "code"}` pair
  HTTP answers with, repeated in the text content. Arguments that fail to
  deserialize are also `isError: true` but carry text alone and no `code`; an
  unknown method or tool name is a JSON-RPC error.
- `TASK_SERVER_ENV=production` is fail-closed without `WORKER_CAPABILITY`,
  `MCP_CAPABILITY`, `APP_MCP_ALLOWED_HOSTS`, `APP_AUTH_ALLOWLIST`,
  `APP_CSRF_TOKEN`, and `APP_ALLOWED_ORIGINS`.
- Unknown `/api/*` paths return the 404 JSON refusal, code `not_found`. Every
  other unknown path falls back to `client/dist/index.html` so the client router
  can restore a deep link.

## Status vocabulary

`draft → ready → wip → done → merged → released`

Sideways from any live status: `blocked`, `cancelled`, `dropped`.
`blocked` returns to `ready`. `wip` may fall back to `ready`.
`released`, `cancelled`, and `dropped` are terminal.

## API

| Method | Path | Authorization |
| --- | --- | --- |
| GET | `/healthz`, `/api/health` | none |
| GET | `/api/session` | read |
| GET | `/api/tasks`, `/api/tasks/{id}` | read |
| POST | `/api/tasks` | human mutation |
| PATCH | `/api/tasks/{id}` | human mutation |
| POST | `/api/tasks/{id}/status` | human mutation |
| GET | `/api/control` | read |
| POST | `/api/merges`, `/api/releases` | human mutation |
| GET | `/api/products`, `/api/products/{id}` | read |
| PUT | `/api/products/{id}` | human mutation |
| POST | `/worker/claim`, `/worker/report` | worker capability |
| POST | `/mcp` | bearer `MCP_CAPABILITY` |
| POST | `/worker/mcp` | bearer `WORKER_CAPABILITY` |

`GET /api/tasks` returns summaries and hides `released` unless `?status=` asks
for a status explicitly; an unknown status is a 400. Single-task responses are
the full task plus `available_transitions`.

`GET /api/control` answers `{ mergeable, pending_merges, releasable }`: the
merge button is live while `mergeable` is non-empty, the release button while
`releasable` carries the product.

## Build / test / run

```sh
npm --prefix client ci
npm --prefix client test
npm --prefix client run build
cargo test --locked
cargo run --locked
```

Tests open their own database, in memory or under a temporary directory. They
never touch a deployed database file.
