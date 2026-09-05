# task-server

A Markdown task ledger with a browser UI, HTTP/MCP access and an append-only
haystack of execution notes. Development runs in a small external loop, using a
fresh agent context for each task.

## Run

```sh
npm --prefix client ci
npm --prefix client run build
APP_DATA_DIR=data/ledger cargo run --locked
```

Open `http://127.0.0.1:3000`. The browser lists active work and history, edits tasks,
shows stopping reasons and milestone evidence, and resumes blocked work.

| Variable | Default / purpose |
|---|---|
| `APP_DATA_DIR` | `data/ledger`, Markdown records |
| `APP_BIND_ADDR` | `127.0.0.1:3000` |
| `APP_STATIC_DIR` | `client/dist` |
| `APP_PROJECTS_DIR` | Optional `<org>/<repo>` catalogue scan root |
| `TASK_SERVER_ENV` | Set `production` behind trusted ingress |
| `APP_CSRF_TOKEN` | Required in production for browser mutation |
| `CLAIM_TTL_SECS` | Claim lifetime; the loop sends heartbeats |

The container listens on port 3000 and stores records below `/app/data/ledger`.
Publish it on loopback behind the existing authenticated/trusted ingress. Worker
and MCP endpoints are trusted-network surfaces, not public endpoints.

## Files and progress

```text
ledger/
  tasks/<task-id>.md
  products/<org>%2F<repo>.md
  runs/<number>.md
  archive/<record-id>.md
  claim_receipts/<record-id>.md
```

Each document has YAML frontmatter and a Markdown body. IDs are encoded in filenames;
metadata retains the original ID. Unknown frontmatter fields survive server edits.
No persistent database or Git daemon is required. A Git repository can be initialized
here for ordinary diff/commit workflows; exclude `.lock` and temporary files.
The server does not automatically commit or push your task content.

Current status (`draft`, `ready`, `wip`, `blocked`, `done`, `cancelled`, `dropped`)
is separate from milestones (`implemented`, `verified`, `reviewed`, `merged`,
`released`). Each milestone records a time, evidence and subject commit. Changed
commits require fresh applicable evidence; prior evidence remains history.
`done` means the requested task outcome is complete, not merely that code was written.

Reads reflect hand edits. Coordinate concurrent editing with execution, and stop
writers for bulk changes. Only one server opens a ledger directory at a time.
Individual record replacement is atomic; there is no general multi-file transaction.

## Execution

`bin/task-loop --help` describes the standalone Python loop. It claims one ready task,
resolves a task worktree, starts a fresh `codex exec`, renews the lease, records the
result and appends a haystack note. The agent uses the installed development skills;
review and fixes are part of that delivery, not additional server-generated tasks.
Use `--once` for a single attempt and `--loop` for continuous execution.

Agent failure, malformed output and timeout become blocked work with saved logs.
An expired lease is visible as interrupted work. Resuming a task retains its known
worktree and milestones, including dirty work created by an earlier attempt.
Unsent results are journaled and retried before another task is taken.

The loop does not grant release or deployment authority. Put the intended outcome
and relevant authorization in the task. It must report incomplete work as blocked,
even when an earlier implementation step succeeded.

## Migration from SQLite

Take an SQLite backup first (SQLite backup API includes committed WAL data). Keep the
old database and image for rollback, then import into a **new** directory:

```sh
bin/task-data import-sqlite /backups/task-server.db /data/ledger-new
```

The source is opened read-only and copied to a consistent in-memory snapshot.
All original task, product, run and claim-receipt columns are retained; additional
tables and schema information are archived. Existing subtask records remain
accessible history and are not claimable by the new loop. Completed work remains
completed; unfinished historical stages are recorded as milestones with an explicit
resume-needed state. Historical milestone timestamps may be estimates and say so.

Check record counts and the UI against the old server before pointing `APP_DATA_DIR`
at the new directory. Stop the old writer before the final import and cutover.
No automatic SQLite-to-Markdown conversion happens during ordinary startup.

## Backup and restore

Create a consistent generation from the running server:

```sh
bin/task-data snapshot --server http://127.0.0.1:3000 --output-dir /backups/task-server
bin/task-data restore /backups/task-server/ledger-TIMESTAMP.tar.gz /data/restored-ledger
```

Snapshots include tasks, catalogue, haystack, read receipts and migration history.
The archive contains a SHA-256 manifest. Restore validates all entries and checksums
before publishing a new directory. Open that directory with a separate server to
verify task history and unread haystack counts. It does not overwrite a live ledger.

For R2, install the AWS CLI and configure `R2_ENDPOINT`, `R2_BUCKET`,
`R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, and optional `R2_PREFIX` (default
`task-server`) outside Git. Then use `snapshot --upload` or `upload <archive>`.
Each generation has a unique timestamp; uploads do not synchronize deletions.
R2 uses its S3 endpoint and region `auto` ([Cloudflare documentation](https://developers.cloudflare.com/r2/examples/aws/aws-cli/)).
Without credentials, local snapshots and restore remain usable; no remote backup
is claimed. Configure retention on the backup destination to suit available space.

## API

The browser uses `/api/session`, `/api/tasks`, `/api/tasks/{id}`,
`/api/tasks/{id}/status`, `/api/closed`, `/api/products` and `/api/runs`.
MCP CRUD remains available at `/mcp`. `/worker/claim`, `/worker/heartbeat`,
`/worker/report` and `/worker/runs` serve the small loop. `/worker/snapshot`
exports a consistent backup generation. Old control-plane issuing endpoints are
retired: review, merge and rework no longer create separate task trees.

Haystack readers can continue using `/api/runs/next` and `/api/runs/{id}/read`.
Mark a run read after its downstream wiki update is safely stored; keep the cursor
on the server so reader restarts do not lose unread work.

## Verification

See [AGENTS.md](AGENTS.md) for build/test commands and [DESIGN.md](DESIGN.md) for
UI behavior. Tests isolate data and agent execution from deployed work.
