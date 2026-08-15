# Task Server

Task control plane. One Axum process serves the JSON API and the compiled
Svelte Task Card UI. A single sqlite database is the source of truth: the
transaction boundary is a sqlite transaction. Workers claim and report over
HTTP; humans review a card and move it to the next status.

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

Reads need an allowlisted `X-Auth-User`. Mutations additionally need a matching
`Origin` and `X-CSRF-Token`. Worker routes need `X-Worker-Capability` instead,
and nothing else grants them. Tasks are never physically deleted; discard one by
moving it to `cancelled` or `dropped`.

## Merging and releasing

The last two steps of a task are not buttons a human presses on the status API;
they are earned.

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
| `APP_DB_PATH` | `data/task-server.db` | SQLite database. Created with its parent directory on first start. |
| `APP_BIND_ADDR` | `127.0.0.1:3000` | HTTP listener. Keep loopback unless you are sure you want otherwise. |
| `CLAIM_TTL_SECS` | `3600` | Lease lifetime for a claim. |
| `WORKER_CAPABILITY` | `dev-worker-capability` | Shared secret for `/worker/*`. Identity headers never substitute. |
| `APP_AUTH_ALLOWLIST` | `miyabi` | Comma-separated identities accepted as `X-Auth-User`. |
| `APP_CSRF_TOKEN` | `dev-csrf` | Required on human mutation as `X-CSRF-Token`. |
| `APP_ALLOWED_ORIGINS` | (unset) | Comma-separated origins accepted on mutation. Outside production, loopback origins are accepted when this is unset. |
| `APP_STATIC_DIR` | `client/dist` | Directory of the production frontend. |
| `TASK_SERVER_ENV` | (unset) | Set to `production` to require the four secrets above and drop the development identity. |
| `RUST_LOG` | `info` | `tracing-subscriber` filter, for example `task_server=debug,tower_http=debug`. |

With `TASK_SERVER_ENV=production` the process refuses to start unless
`WORKER_CAPABILITY`, `APP_AUTH_ALLOWLIST`, `APP_CSRF_TOKEN`, and
`APP_ALLOWED_ORIGINS` are all set.

## Repository structure

```text
.
├── client/             # Svelte 5 Task Card UI
├── src/                # Axum router, sqlite store, status machine
├── tests/              # Contract tests against the public API
├── Cargo.toml
├── DESIGN.md
└── rust-toolchain.toml
```

Unknown `/api/*` paths return 404. Other unknown paths fall back to
`client/dist/index.html` so the client router can restore a deep link.

## License

MIT. See [LICENSE](LICENSE).
