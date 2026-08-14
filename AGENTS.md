# task-server

Household task control plane. Single writer over a dedicated tasks git work
tree. Version 0.1.0.

## Invariants

- Git is truth. The transaction boundary is a git commit.
- Tasks root is `TASKS_GIT_DIR`. Do not hardcode a household path.
- Claim returns a lease only after the claim commit.
- Report accept and the status flip to `awaiting_user` are the same commit.
- Push and notify are not in that commit; they go to `$TASKS_GIT_DIR/.outbox`.
- Rewrite YAML frontmatter only. Markdown body bytes after the closing `---`
  fence stay bit-identical.
- Status truth is the `status:` field only. Do not encode state in the path.
- Listen on `127.0.0.1` by default (`APP_BIND_ADDR` may override).
- Worker routes require `X-Worker-Capability` equal to the configured secret.
  An identity header alone does not authenticate.
- Human identity comes from ingress (`X-Auth-User` or `Tailscale-User-Login`).
  The browser does not mint identity. `TASK_SERVER_ENV=production` is
  fail-closed without `WORKER_CAPABILITY`, `APP_AUTH_ALLOWLIST`,
  `APP_CSRF_TOKEN`, `APP_ALLOWED_ORIGINS`, and `NTFY_URL`.
- Human mutation requires an allowlisted ingress identity, a matching
  `Origin`, and `X-CSRF-Token`. Worker capability is not sufficient.
- Action names come from `config/actions.json` (or `ACTION_TABLE_PATH`).
- After report, pending outbox intents are POSTed to `NTFY_URL`. 2xx marks
  `delivered`. Notify failure does not roll back the status commit. Startup
  flushes the outbox again.
- Unexpired running claims are exclusive. An expired lease may be reclaimed
  with a new `claim_id`. A report with a stale `claim_id` is rejected.
- Clock is injectable. Default claim TTL is 3600 seconds (`CLAIM_TTL_SECS`).
- `available_actions(status)` and action translation share one table.
- If `target_space` is `tasks`, `ready → awaiting_user` is allowed without
  `running` (self-service).

## Status vocabulary

`draft → ready → running → awaiting_user → done → release_requested → released | release_failed`

Sideways: `blocked` / `cancelled` / `dropped`.

Required fields:

- ready: `next_action`; development: `target_space` or `product_id`
- running: `claim_id`, `claimed_at`, `worker`, `claim_expires_at`
- awaiting_user: `commit_sha`, `verification`
- release_requested: `release_repo`, `release_sha`, `bump` in {patch,minor,major}
- released: the triple plus `release_tag`
- release_failed: the triple plus a non-empty `failure`

Datetimes: `YYYY-MM-DDTHH:MM:SSZ` or `YYYY-MM-DDTHH:MM:SS±HH:MM`.

## Build / test / run

```sh
npm --prefix client ci
npm --prefix client test
npm --prefix client run build
cargo test --locked
TASKS_GIT_DIR=/path/to/tasks-clone cargo run --locked
```

The store expects `$TASKS_GIT_DIR/projects/queue/tasks/<id>.md`. Tests create
their own `git init` fixtures; they do not use a live household clone.
