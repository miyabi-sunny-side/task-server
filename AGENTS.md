# task-server

One Axum process serves a Markdown task ledger, JSON API, MCP and the Svelte UI.

## Ownership

- `APP_DATA_DIR` (default `data/ledger`) is the persistent truth. Collections are
  `tasks`, `products`, `runs`, `archive`, `claim_receipts`; one YAML-frontmatter
  Markdown file per record. Body text follows the closing fence. Unknown metadata
  survives updates. Filenames percent-encode UTF-8 IDs; products remain `org/repo`.
- `ledger::Store` owns reading, validation, one-process exclusion and atomic file
  replacement. `transaction` serializes operations; it is not multi-file rollback.
  Task state and its milestones belong to the same file.
  New completion reports live once in a run Markdown body; task/milestone
  report IDs reference it. The accepted run intent is replayed before task
  mutations/lease expiry after an interrupted task write. Do not introduce SQLite
  persistence or a second authoritative cache.
- Reads load files again, so hand edits are visible. Coordinate hand edits with
  running writers; stop the service before bulk edits or restoring a snapshot.
- `task` owns current status, claims and milestones. `runs` owns the haystack and
  reading receipts. HTTP and MCP call the same domain functions.
- The server does not launch agents or create review/merge/release subtasks.
  `bin/task-loop` executes one fresh agent context per task; delivery skills own
  implementation, validation, independent review and fixes inside that context.

## Progress and recovery

Current status is `draft`, `ready`, `wip`, `blocked`, `done`, `cancelled`, or
`dropped`. Reaching an implementation/review/merge milestone does not itself mean
that the entire task is done. Milestones carry their evidence and subject commit.
A changed commit invalidates dependent evidence while retaining its history.

A claim is a lease, renewed by heartbeat. Expired work is visible as blocked,
never silently reported as successful or repeatedly reissued. Resuming blocked
work preserves its recorded progress. A failed agent or merge must leave a reason
and reusable working state. Reports and haystack resends are idempotent.

Legacy SQLite is read only by `bin/task-data import-sqlite`, never by the server.
Migration keeps every original column in `legacy` and preserves archived subtask
history and haystack read receipts. Import and restore publish into a new directory;
existing destinations are never overwritten. Old automatic pipeline rules are retired.

## Network and backup

The default bind is loopback. Production human mutation requires ingress identity
and `X-CSRF-Token`; `APP_CSRF_TOKEN` is required in production. Worker and MCP
surfaces rely on trusted network ingress. Preserve this deployment boundary.

Backups are external: `GET /worker/snapshot` exports all collections under the
writer lock. `bin/task-data` creates checksummed generations and restores to a new
directory. R2 upload uses external credentials, never tracked secrets. A successful
local snapshot is not evidence of a successful remote upload.

## Checks

```sh
npm --prefix client ci
npm --prefix client run check
npm --prefix client test
npm --prefix client run build
npm --prefix client run lint:design
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
python3 -m unittest discover -s tests -p 'test_*.py'
```

Tests use temporary ledgers and stub agents/HTTP servers, never deployed tasks.
Frontend appearance and interaction follow root `DESIGN.md`.
