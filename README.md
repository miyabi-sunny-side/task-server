# Task Server

Task control plane. One Axum process serves the JSON API, two MCP endpoints, and
the compiled Svelte Task Card UI. A single sqlite database is the source of
truth: the transaction boundary is a sqlite transaction. Workers claim and
report over HTTP or MCP; humans review a card and move it to the next status.

[`DESIGN.md`](DESIGN.md) is the styling authority (Sumi dark, Kinari light, teal
accent). [`AGENTS.md`](AGENTS.md) lists runtime invariants.

## Prerequisites

- Rust 1.96.0 (the checked-in toolchain file selects it through `rustup`)
- Node.js 24 LTS and npm

The only JavaScript lockfile is `client/package-lock.json`. Run npm from
`client/` and use `npm ci` for reproducible installs.

## Quick start

```sh
cd client
npm ci
npm run build
cd ..
APP_DB_PATH=data/task-server.db cargo run --locked
```

Open <http://127.0.0.1:3000>. The process listens on loopback by default and
creates the database (and its parent directory) on first start.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/healthz` | plain-text liveness: `ok` |
| GET | `/api/health` | `{"status":"ok"}` |
| GET | `/api/session` | the caller's identity and CSRF token |
| GET | `/api/tasks` | task summaries; `released` is hidden unless `?status=` asks for it |
| POST | `/api/tasks` | create a task in `draft`, returns 201 |
| GET | `/api/tasks/{id}` | Task Card: the task plus `available_transitions` |
| PATCH | `/api/tasks/{id}` | edit `title`, `body`, `product_id`, `priority`, `branch` |
| POST | `/api/tasks/{id}/status` | move the task to `{"status": "..."}`; `merged` and `released` are refused |
| GET | `/api/control` | `{ mergeable, pending_merges, releasable }` |
| POST | `/api/merges` | `{"task_id": "..."}` issues the merge task, returns 201 |
| POST | `/api/releases` | `{"product_id": "...", "tag": "..."}` releases everything merged |
| GET | `/api/products` | product list |
| GET | `/api/products/{id}` | one product |
| PUT | `/api/products/{id}` | create or replace a product |
| POST | `/worker/claim` | lease the next ready task |
| POST | `/worker/report` | report a commit, and `checks`, against a lease |
| POST | `/mcp` | MCP over Streamable HTTP: the catalogue and the task lifecycle |
| POST | `/worker/mcp` | MCP over Streamable HTTP: claim and report |

Reads need an allowlisted `X-Auth-User`. Mutations additionally need a matching
`Origin` and `X-CSRF-Token`. Worker routes need `X-Worker-Capability` instead,
and nothing else grants them. The two MCP endpoints are opened by a bearer
capability alone — see [MCP](#mcp). Tasks are never physically deleted; discard
one by moving it to `cancelled` or `dropped`.

## The product catalogue

The `products` table is the register of product identity. Anything that names a
product — a task, a merge, a release — means a row in it.

Filing work and curating the catalogue are two different moments, so
`POST /api/tasks` accepts a `product_id` the catalogue has never heard of and
the task starts in `draft` as usual. Promoting it is where identity is required:
`POST /api/tasks/{id}/status` with `ready` answers 409 when the product is not
catalogued, or when the task has no `product_id` at all, and the task stays
where it was. Add the product with `PUT /api/products/{id}` and the same
promotion goes through.

Refusals that come from the server's own domain carry a stable `code` next to
their human `error` message, so an automated client branches on the reason
rather than on the prose:

```json
{
  "error": "product 'org/repo' is not in the product catalogue, so task t-1 cannot become ready; add it first with PUT /api/products/org/repo",
  "code": "product_not_catalogued"
}
```

The codes are `unauthorized`, `forbidden`, `not_found`, `claim_mismatch`,
`invalid`, `conflict`, `product_required`, `product_not_catalogued`,
`frontmatter`, `io`, and `db`. An unknown `/api/*` path answers in the same
shape, as a 404 with code `not_found`.

One kind of failure is outside that contract: a request body that is not valid
JSON is rejected by the web framework before any handler runs, so it comes back
as `400` with a plain-text explanation and no `code` to branch on.

Set `APP_PRODUCTS_SEED` to fill the catalogue at startup from a JSON roster:

```json
[
  {
    "id": "org/repo",
    "repository": "https://github.com/org/repo",
    "description": "one line",
    "releases": true
  },
  { "id": "org/other", "repository": "https://github.com/org/other" }
]
```

`description` defaults to empty and `releases` to `true`. Each entry is an
upsert keyed on `id`, so restarting with the same file adds no duplicates and
editing the file corrects the row in place. `created_at` survives either way;
`updated_at` is stamped on every seed, including a re-run of an unchanged file.
Nothing is ever removed by a seed. A missing file, JSON that does not parse, or
an id that is not `org/repo` stops the startup with nothing written.

## Merging and releasing

The last two steps of a task are not buttons a human presses on the status API;
they are earned.

`POST /api/merges` is the only issuer of an `instant:merge` task, and that is an
invariant of the domain rather than a rule of one transport: task registration
files ordinary work only, so `POST /api/tasks` refuses `"kind": "instant:merge"`
with 400 and code `invalid`, and the MCP `task_create` tool has no `kind`
argument at all. A hand-made merge would be a merge with no target — claimed
ahead of every other task, impossible to report, and so a standing block on the
queue.

A task becomes **mergeable** once a worker reported it `done` with a branch and
a commit, and no live merge already targets it. `POST /api/merges` then issues
one `instant:merge` task that inherits the target's product, branch, and commit,
starts in `ready`, and is claimed ahead of ordinary work. Only one live merge may
target a task, so a second issue answers 409. Cancelling or dropping the attempt
frees the target again, and the retry is issued under its own id: `merge:<id>`
first, then `merge:<id>~2`, `~3`, and so on.

The worker that claims it rebases the branch onto the main line and reports back
with the checks it ran:

```jsonc
{
  "claim_id": "...",
  "commit_sha": "abc1234",
  "verification": "cargo test",
  "checks": [{ "name": "cargo test", "exit_code": 0 }]
}
```

A merge report with no checks, or with any non-zero `exit_code`, is refused and
changes nothing — including a repeat report against a merge that already landed.
When every check passed, the merge finishes and its target moves to `merged` in
the same transaction.

Merged work then piles up per product. For a product with `releases` set,
`GET /api/control` reports how much is waiting, and `POST /api/releases` stamps
every merged task of that product with one `release_tag` and moves them all to
`released`. A product that does not release, or one with nothing merged,
answers 409.

Stop the service with <kbd>Ctrl</kbd>+<kbd>C</kbd>.

## MCP

The same control plane also speaks MCP, over Streamable HTTP, from the same
process and against the same sqlite. Two endpoints, each opened by its own
bearer capability:

| Endpoint | Authorization | Tools |
| --- | --- | --- |
| `POST /mcp` | `Bearer $MCP_CAPABILITY` | `product_list`, `task_create`, `task_get`, `task_list`, `task_update`, `task_set_status` |
| `POST /worker/mcp` | `Bearer $WORKER_CAPABILITY` | `task_claim`, `task_report` |

Point a client at `http://127.0.0.1:3000/mcp` with that `Authorization` header;
the `initialize` handshake hands back an `Mcp-Session-Id` the client carries on
every later request.

The bearer is the whole gate. The ingress identity, `Origin`, and CSRF checks
the human API applies are not run for MCP, and neither capability opens the
other's endpoint. A request that presents the wrong capability, or none, is
answered `401` with the usual `{"error", "code"}` body and never reaches
JSON-RPC: a caller that did not get past the door has no session to answer in.

Curating the catalogue, issuing a merge, and cutting a release stay off MCP on
purpose — they are the human decisions the rest of this README describes, and
they are made over HTTP. There is no delete tool either, for the same reason
there is no delete route. `task_create` therefore files ordinary work and takes
no `kind`, and `task_set_status` refuses `merged` and `released` with the same
code and for the same reason the HTTP status route does — one domain function
answers both, so neither transport can become a way around the other.

A refusal the domain owns is not a protocol failure, so it comes back as a tool
result with `isError: true`. Its `structuredContent` is the same
`{"error", "code"}` pair the HTTP API answers with, repeated verbatim in the
text content, so a client branches on the code rather than on the prose:

```json
{
  "isError": true,
  "structuredContent": {
    "error": "product 'org/repo' is not in the product catalogue, so task t-1 cannot become ready; add it first with PUT /api/products/org/repo",
    "code": "product_not_catalogued"
  }
}
```

Two failures fall outside that shape. Arguments that do not deserialize also
answer `isError: true`, but with a plain-text explanation and no `code` to
branch on; an unknown method or tool name is a JSON-RPC error, not a result.

Both endpoints validate the `Host` header, which is what stops DNS rebinding: a
page served under an attacker's name can re-resolve that name to `127.0.0.1` and
then reach a loopback server from its own origin, with no CORS preflight in the
way. The bearer capability is no answer to that on its own — the development
default is a published constant, printed in this README — so the allowlist is on
by default and accepts loopback `Host` values only. A request carrying any other
`Host` is answered `403` before it reaches JSON-RPC.

A deployment reached under its own name declares that name in
`APP_MCP_ALLOWED_HOSTS`, which replaces the loopback default with exactly the
authorities listed (so include `127.0.0.1` there if loopback clients must still
work). Behind a reverse proxy, list the `Host` the proxy forwards, not the
internal address. `TASK_SERVER_ENV=production` refuses to start without it. Treat
`MCP_CAPABILITY` and `WORKER_CAPABILITY` as secrets in any case.

## Importing the markdown queue

Before sqlite the queue was a directory of markdown files with YAML
frontmatter — a live queue and an archive. `import-markdown` is the one
subcommand, and it reads them into the database; with no arguments the binary
is still the server.

```sh
APP_DB_PATH=data/task-server.db \
  cargo run --locked -- import-markdown --live queue --archive archive
```

Either directory alone is a valid import and omitting both is a usage error.
Each is read recursively for `*.md`, so an archive split into year directories
needs no flattening and no other layout is assumed. The task id is the file
stem: `queue/t-101.md` becomes task `t-101`, and so the same stem under both
directories is two files claiming one row rather than a merge. The database is
whichever one `APP_DB_PATH` names, created with its parent directory if it is
not there yet, so importing into a fresh file is a normal first run.

The markdown itself is only ever read. The import writes, moves, and deletes
nothing under either directory; removing the old queue afterwards is a separate
decision for whoever ran it.

The run is all or nothing. Every file is parsed and checked first, then a
single transaction writes the lot, so one unreadable file, one file with no
`title` or `status`, one duplicate id, one status nobody can map, or one row
already in the database with different content leaves the database exactly as it
was. Every file the run refused is listed at once rather than one per
attempt, and the exit code is non-zero:

```text
import refused (1 problem(s)); nothing was written
  queue/t-103.md: unknown v0.1 status 'frobnicated'
```

A run that goes through prints what it did:

```text
read 3 file(s): 2 live, 1 archive
inserted 3: wip 1, done 1, merged 1
skipped 0 (already imported, unchanged)
warning: 2 product(s) not in the catalogue: example/other, example/repo
```

Re-running the same input changes nothing: a file that would write the row
already sitting under its id is skipped, so the second run over those three
files reports `inserted 0` and `skipped 3`, and no row is rewritten. Editing a
file that was already imported is not an update — it is the conflict above, and
it stops the whole run.

The v0.1 statuses land in the v0.2 vocabulary like this:

| v0.1 status | imported as |
| --- | --- |
| `running` | `wip` |
| `awaiting_user` | `done` |
| `done` | `merged` |
| `release_requested` | `merged` |
| `release_failed` | `merged` |
| `draft`, `ready`, `released`, `blocked`, `cancelled`, `dropped` | unchanged |

`done` is the one that carries a decision rather than a rename: in v0.1 it meant
the human had accepted the work and it was over, which is `merged` here.
Importing it as `done` would raise every finished task of the old queue as a
merge candidate. A status neither the table nor the vocabulary knows refuses the
import instead of being quietly dropped.

Frontmatter that has a column of its own keeps it: `title`, `commit_sha`,
`verification`, `release_tag`, and the product, which is `target_space` or
`product_id` when there is no `target_space`. The product reaches its column
only when it reads as `org/repo`; a queue old enough to predate that convention
carries names like `tasks`, and refusing those would mean editing archived
history to migrate it. Such a row imports with no product, and the value it did
carry stays in the folded block below. Everything else is folded onto the
end of the body as one marked YAML block, led by the pre-mapping status, so
nothing that was written down is lost and the mapping can be read backwards:

````text
# wire up the health probe

Original body, kept verbatim.

---

## Imported v0.1 metadata

```yaml
status: running
area: development
tags:
- infra
```
````

Imported rows are ordinary work — `normal` kind, priority 0, no branch and no
claim — and their `created_at` and `updated_at` record when the import ran.

A product the imported rows name but the catalogue does not carry is a warning
and never a refusal: the import registers no product it was not given. The
catalogue gate is what asks for it later, so add the product with
`PUT /api/products/{id}` before promoting an imported task to `ready`. A row
that kept a pre-convention product reference is counted on its own line and
reaches the same gate, which refuses it with `product_required` until someone
decides which product it belongs to:

```text
read 3 file(s): 1 live, 2 archive
inserted 3: done 1, merged 1, released 1
skipped 0 (already imported, unchanged)
2 task(s) kept a legacy product reference in the body (product_id left unset)
warning: 1 product(s) not in the catalogue: example/repo
```

## Development

Terminal 1, from the repository root:

```sh
cargo run
```

Terminal 2:

```sh
cd client
npm ci
npm run dev
```

Open <http://127.0.0.1:5173>. Vite proxies `/api` to `http://127.0.0.1:3000`.

The UI sends `X-Auth-User` and `X-CSRF-Token`. Defaults are `miyabi` /
`dev-csrf` (override with localStorage keys `task-server:user` and
`task-server:csrf`).

## Verify changes

```sh
npm --prefix client ci
npm --prefix client run format:check
npm --prefix client run check
npm --prefix client test
npm --prefix client run build
npm --prefix client run lint:design
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked --release
```

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `APP_DB_PATH` | `data/task-server.db` | SQLite database, for the server and for `import-markdown` alike. Created with its parent directory on first use. |
| `APP_BIND_ADDR` | `127.0.0.1:3000` | HTTP listener. Keep loopback unless you are sure you want otherwise. |
| `CLAIM_TTL_SECS` | `3600` | Lease lifetime for a claim. |
| `WORKER_CAPABILITY` | `dev-worker-capability` | Shared secret for `/worker/*`, including `/worker/mcp`. Identity headers never substitute. |
| `MCP_CAPABILITY` | `dev-mcp-capability` | Bearer secret for `/mcp`. Kept apart from the worker capability so a worker credential never opens task CRUD. |
| `APP_MCP_ALLOWED_HOSTS` | (unset) | Comma-separated `Host` authorities the MCP endpoints answer to, replacing the loopback-only default that stops DNS rebinding. Unset means loopback only. Required in production. |
| `APP_AUTH_ALLOWLIST` | `miyabi` | Comma-separated identities accepted as `X-Auth-User`. |
| `APP_CSRF_TOKEN` | `dev-csrf` | Required on human mutation as `X-CSRF-Token`. |
| `APP_ALLOWED_ORIGINS` | (unset) | Comma-separated origins accepted on mutation. Outside production, loopback origins are accepted when this is unset. |
| `APP_STATIC_DIR` | `client/dist` | Directory of the production frontend. |
| `APP_PRODUCTS_SEED` | (unset) | Path to a JSON product roster upserted into the catalogue at startup. Unset means the catalogue is curated over the API alone. |
| `TASK_SERVER_ENV` | (unset) | Set to `production` to require the six variables listed below and drop the development identity. |
| `RUST_LOG` | `info` | `tracing-subscriber` filter, for example `task_server=debug,tower_http=debug`. |

With `TASK_SERVER_ENV=production` the process refuses to start unless
`WORKER_CAPABILITY`, `MCP_CAPABILITY`, `APP_MCP_ALLOWED_HOSTS`,
`APP_AUTH_ALLOWLIST`, `APP_CSRF_TOKEN`, and `APP_ALLOWED_ORIGINS` are all set.

## Repository structure

```text
.
├── client/             # Svelte 5 Task Card UI
├── src/                # Axum router, MCP endpoints, sqlite store, status machine
├── tests/              # Contract tests against the public API
├── Cargo.toml
├── DESIGN.md
└── rust-toolchain.toml
```

Unknown `/api/*` paths return the 404 JSON refusal. Other unknown paths fall
back to `client/dist/index.html` so the client router can restore a deep link.

## License

MIT. See [LICENSE](LICENSE).
