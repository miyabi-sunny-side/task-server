# Task Server

Household task control plane. One Axum process serves the JSON API and the
compiled Svelte Task Card UI. Git is the source of truth: the transaction
boundary is a commit. Workers claim and report over HTTP; humans review a card
and press an action.

[`DESIGN.md`](DESIGN.md) is the styling authority (Sumi dark, Kinari light, teal
accent). [`AGENTS.md`](AGENTS.md) lists runtime invariants.

## Prerequisites

- Rust 1.96.0 (the checked-in toolchain file selects it through `rustup`)
- Node.js 24 LTS and npm
- `git` on `PATH` (the server is a single writer on a dedicated clone)

The only JavaScript lockfile is `client/package-lock.json`. Run npm from
`client/` and use `npm ci` for reproducible installs.

## Quick start

```sh
cd client
npm ci
npm run build
cd ..
TASKS_GIT_DIR=/path/to/tasks-clone cargo run --locked
```

Open <http://127.0.0.1:3000>. The process listens on loopback by default.

- `GET /healthz` — plain-text liveness: `ok`
- `GET /api/health` — `{"status":"ok"}`
- `GET /api/tasks` — task list (requires allowlisted `X-Auth-User`)
- `GET /api/tasks/:id` — Task Card
- `POST /api/tasks/:id/actions/:action` — human action (allowlist + CSRF)
- `POST /worker/claim` — worker lease (requires `X-Worker-Capability`)
- `POST /worker/report` — worker report (same header)

Stop the service with <kbd>Ctrl</kbd>+<kbd>C</kbd>.

## Development

Terminal 1, from the repository root:

```sh
TASKS_GIT_DIR=/path/to/tasks-clone cargo run
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
| `TASKS_GIT_DIR` | (unset) | Dedicated git work tree. Task files live at `projects/queue/tasks/<id>.md`. Domain writes fail if this is not a git repo. |
| `APP_BIND_ADDR` | `127.0.0.1:3000` | HTTP listener. Keep loopback unless you are sure you want otherwise. |
| `CLAIM_TTL_SECS` | `3600` | Lease lifetime for a claim. |
| `WORKER_CAPABILITY` | `dev-worker-capability` | Shared secret for `/worker/*`. Identity headers never substitute. |
| `APP_AUTH_ALLOWLIST` | `miyabi` | Comma-separated identities accepted as `X-Auth-User`. |
| `APP_CSRF_TOKEN` | `dev-csrf` | Required on human mutation as `X-CSRF-Token`. |
| `APP_STATIC_DIR` | `client/dist` | Directory of the production frontend. |
| `RUST_LOG` | `info` | `tracing-subscriber` filter, for example `task_server=debug,tower_http=debug`. |

A durable outbox is stored at `$TASKS_GIT_DIR/.outbox`. Notify and push are not
part of the status commit.

## Repository structure

```text
.
├── client/             # Svelte 5 Task Card UI
├── src/                # Axum router, git store, status machine
├── tests/              # Contract tests against the public API
├── Cargo.toml
├── DESIGN.md
└── rust-toolchain.toml
```

Unknown `/api/*` paths return 404. Other unknown paths fall back to
`client/dist/index.html` so the client router can restore a deep link.

## License

MIT. See [LICENSE](LICENSE).
