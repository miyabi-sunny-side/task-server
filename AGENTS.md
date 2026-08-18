# task-server

Task control plane. One Axum process owns one sqlite database and serves the
JSON API, the MCP endpoints, and the compiled client.

## Invariants

- SQLite is truth. The transaction boundary is a sqlite transaction, taken
  `IMMEDIATE` so concurrent writers serialize at `BEGIN`.
- The database lives at `APP_DB_PATH` (default `data/task-server.db`).
  Migrations run at open and are keyed on `PRAGMA user_version`.
- A database on disk is in WAL or it does not open. `Db::open` reads
  `journal_mode` back and refuses anything else, because continuous backup
  follows the write-ahead log and a replica of a `delete` mode database would
  quietly stop tracking the truth. `busy_timeout` is read back the same way.
- What is exempt from that is decided from the path asked for, never from what
  sqlite reports afterwards: sqlite names a temporary database exactly as it
  names an in-memory one, so a blank `APP_DB_PATH` would otherwise open a
  scratch database that is neither replicated nor persisted. A blank or
  whitespace-only path is refused outright. Only the literal `:memory:` — and
  the internal in-memory open the tests use — is exempt; URI spellings such as
  `file::memory:` are treated as files and so fail the WAL read-back.
- The write-ahead log is left to sqlite. `wal_autocheckpoint` is never set and no
  checkpoint is ever forced: trimming the log is sqlite's business, and where the
  log is replicated the replicator has to have read a frame before it goes.
- Backup is a sidecar, never the server. The process replicates nothing and
  starts no Litestream, so an image given no backup configuration starts exactly
  as it always did. `deploy/litestream.yml.example` reaches every credential,
  bucket, and endpoint through the environment and carries none of them.
- HTTP is the only way in, carrying both the JSON API and the MCP endpoints.
  There is no file store and no git side effect.
- There is no physical delete. Discarding a task is a transition to
  `cancelled` or `dropped`, so the row stays auditable.
- Product ids are `org/repo`, never a path. Task ids are one path segment.
- The `products` table is the register of product identity. A task may be
  created with a `product_id` that is not in it, but it may not be promoted:
  `ready` is refused with 409 and code `product_not_catalogued` when the product
  is uncatalogued, or `product_required` when the task has no `product_id`. With a
  project tree configured the remedy for `product_not_catalogued` is the tree (no
  clone sits at that id) or a corrected `product_id`, never a hand-written row. The
  gate fires on the transition only, so a row that is already `ready` or beyond
  is never demoted, and `available_transitions` still offers `ready`.
- `APP_PROJECTS_DIR` derives the catalogue from the `<org>/<repo>` tree on disk
  at startup, in one transaction: a git repository with an `origin` remote is a
  product, and its id is the local placement rather than the remote's owner. A row
  matching the tree is not rewritten, so `updated_at` is not stamped by a restart.
  An empty walk changes nothing and warns — including on an empty catalogue, since
  the warning is about the walk; an unreadable root fails the startup. Unset means
  nothing is walked. The retired `APP_PRODUCTS_SEED` refuses the start.
- A git repository is git's own definition, not "it has a config": `HEAD` reading
  as a ref or an object name, an object store, and `refs`. `releases` needs a
  whole strict SemVer tag (`v` optional), so `01.2.3` or `1.2.3-` is not one.
- The walk never reads outside `APP_PROJECTS_DIR`. Every path is canonicalised and
  has to stay under the root: a `.git` symlink, a `gitdir:` pointer, or a
  `commondir` leading out is skipped as `outside_root` and counted, and README,
  workflow, and refs paths that resolve out are read as absent. Worktree and
  submodule pointers are still followed while they stay inside.
- A product whose working copy left the tree is **archived, never deleted**:
  `products.archived_at` is set, the row stays so the tasks that named it keep
  resolving, and `ready` is refused with code `product_archived` — distinct from
  `product_not_catalogued`, because the remedy is restoring the clone. The next
  walk that finds the directory again clears the mark. `PUT /api/products/{id}`
  never sets or clears it.
- `task-server import-markdown --live <DIR> [--archive <DIR>]` is the only
  subcommand; no arguments is the server. Either directory alone is valid, both
  omitted is a usage error, and each is read recursively for `*.md` into
  `APP_DB_PATH`. The import is all or nothing: every file is parsed and checked
  first, then one transaction writes the lot, so an unreadable file, a missing
  `title` or `status`, a duplicate id, an unmappable status, or a row already
  in the database with different content writes nothing and exits non-zero with
  every refused file listed. The markdown is only read — the import never
  writes, moves, or deletes a file under either directory.
- A task id is the file stem, so the same stem in the live queue and the
  archive is two files claiming one row, not a merge. A file that would write
  the row already under its id is skipped, so re-running the same input leaves
  every row as it was.
- Every v0.1 status reaches the v0.2 vocabulary — `running → wip`,
  `awaiting_user → done`, `done`, `release_requested`, and `release_failed` →
  `merged`, everything else under its own name — and an unknown one refuses the
  import rather than being dropped. `done` is the decision, not a rename: v0.1
  `done` was accepted, finished work, and importing it as `done` would raise
  every finished task of the old queue as a merge candidate.
- Frontmatter the schema has no column for is folded onto the end of the body
  as one `## Imported v0.1 metadata` YAML block, led by the pre-mapping status,
  so nothing written down is lost and the mapping reads backwards. Imported rows
  are `normal` tasks at priority 0 with no branch and no claim. A product the
  import names that the catalogue lacks is a warning, never a refusal and never
  an auto-created row; the `ready` gate is what asks for it later.
- A product reference that does not read as `org/repo` is not a refusal either.
  It leaves `product_id` unset, keeps its value in the folded block, and is
  counted on its own summary line — a queue older than the convention migrates
  without rewriting its archive, and the `ready` gate decides the product later.
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
- Human mutation requires an ingress identity and `X-CSRF-Token`. The identity
  is taken at face value and `Origin` is not read: which clients reach this
  server is the reverse proxy's decision, and the token is what a cross-site
  page cannot produce. Worker capability is not sufficient.
- MCP is a second transport, not a second domain. `/mcp` and `/worker/mcp` are
  Streamable HTTP endpoints in the same process, and every tool decodes its
  arguments and calls `src/task.rs` or `src/product.rs`. The transition table,
  the catalogue gate, and the SQL are never duplicated there.
- Each MCP endpoint has its own bearer: `MCP_CAPABILITY` for `/mcp`,
  `WORKER_CAPABILITY` for `/worker/mcp`. A worker credential never opens task
  CRUD. The check runs before rmcp sees the request, so a missing or mismatched
  bearer is 401 and gets no JSON-RPC answer at all.
- Ingress identity, `Origin`, and CSRF are not applied to MCP; the bearer is the
  whole gate. rmcp's loopback-only `Host` allowlist is switched off with
  `disable_allowed_hosts()`, because this server is reached through a reverse
  proxy that already decides which names it serves and the default would refuse
  the name that proxy forwards.
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
  `MCP_CAPABILITY`, and `APP_CSRF_TOKEN`. Nothing a reverse proxy can decide is
  configured here.
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
