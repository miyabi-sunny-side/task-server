# task-server

Task control plane. One Axum process owns one sqlite database and serves both
the JSON API and the compiled client.

## Invariants

- SQLite is truth. The transaction boundary is a sqlite transaction, taken
  `IMMEDIATE` so concurrent writers serialize at `BEGIN`.
- The database lives at `APP_DB_PATH` (default `data/task-server.db`).
  Migrations run at open and are keyed on `PRAGMA user_version`.
- HTTP is the only way in. There is no file store and no git side effect.
- There is no physical delete. Discarding a task is a transition to
  `cancelled` or `dropped`, so the row stays auditable.
- Product ids are `org/repo`, never a path. Task ids are one path segment.
- `merged` and `released` belong to the control plane. `POST /api/tasks/{id}/status`
  refuses both, and `available_transitions` never offers them; the transition
  table still allows them because the control plane goes through it.
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
- `TASK_SERVER_ENV=production` is fail-closed without `WORKER_CAPABILITY`,
  `APP_AUTH_ALLOWLIST`, `APP_CSRF_TOKEN`, and `APP_ALLOWED_ORIGINS`.
- Unknown `/api/*` paths return 404. Every other unknown path falls back to
  `client/dist/index.html` so the client router can restore a deep link.

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
