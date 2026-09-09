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

The Rust build embeds `client/dist` in the binary, so build the frontend first
for both development and release builds. After changing frontend files, rebuild
the frontend and restart `cargo run` to embed the new assets. For distribution,
run `cargo build --locked --release` after the frontend build and copy only
`target/release/task-server`; no UI directory is needed at runtime.

## Environment variables

All four application settings are optional and are read at startup. Helper CLI
settings are listed separately under [Helper and deployment settings](#helper-and-deployment-settings).

| Variable | Default when unset | Purpose / invalid values | Reader |
| --- | --- | --- | --- |
| `APP_DATA_DIR` | `data/ledger` | Markdown ledger path, relative to the working directory. Missing directories are created. Blank paths, inaccessible storage or an already-locked ledger fail startup; an empty ledger beside a legacy `task-server.db` requires explicit import. | [`src/state.rs`](src/state.rs), [`src/ledger.rs`](src/ledger.rs) |
| `PORT` | `3000` | Decimal TCP port from `1` to `65535`. Empty, non-Unicode, signed, whitespace-padded, nonnumeric or out-of-range values fail startup with a `PORT` error. | [`src/port.rs`](src/port.rs) |
| `LOG_LEVEL` | `info` | Logging verbosity: exactly `off`, `error`, `warn`, `info`, `debug`, or `trace`. Empty, non-Unicode or invalid values (including uppercase and module filters) use `info`. | [`src/logging.rs`](src/logging.rs) |
| `CLAIM_TTL_SECS` | `3600` seconds | Integer from `1` through `86400`, inclusive. A claim and each heartbeat set expiry to now plus this lifetime. Empty, nonnumeric, whitespace-padded or out-of-range values fail startup. | [`src/state.rs`](src/state.rs), [`src/task.rs`](src/task.rs) |

Non-Unicode `APP_DATA_DIR` and `CLAIM_TTL_SECS` values are treated as unset.
The former `RUST_LOG` is ignored; use `LOG_LEVEL`. Build-time Cargo/CI variables
are not runtime application settings.

The server listens on `0.0.0.0:${PORT}` in both native and container runs. Native
runs therefore accept connections on all IPv4 interfaces. `APP_BIND_ADDR` is no
longer read. For example, `PORT=3100 cargo run --locked` serves port 3100.
The container defaults to port 3000 and stores records below `/app/data/ledger`.
When overriding it, pass `-e PORT=3100 -p 127.0.0.1:3100:3100` to `docker run`.
Publish it on loopback behind the existing trusted LAN/tailnet ingress. HTTP
and MCP endpoints rely on that network boundary. The app does not authenticate
callers, require identity headers or provide a browser session API. Claim ownership,
lease checks, state transitions and history remain part of task management.

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

## Explicit product metadata

The product Markdown record is the sole owner of operational product metadata.
The server never discovers repositories, reads repository configuration, or scans
local directories. `product_rescan` is removed and
`POST /api/products/rescan` returns 410. Directory changes and restarts cannot add,
archive, revive, or change products. The browser menu's product list shows all
registered products, including archived entries and unknown release policies.

| Field | Meaning |
|---|---|
| `id` | Stable `org/repo` key used by existing tasks; never renamed by a metadata update |
| `repository` | Required canonical repository URL/address, independent of ID and local placement |
| `description` | Human description, default empty |
| `local_path` | Optional absolute checkout or bare repository path on the executing machine; `null` means unconfigured |
| `releases` | `true`: releases supported; `false`: do not publish; `null` or missing: policy unknown |
| `archived`, `archived_at` | Explicit archive state and timestamp; registration/update cannot revive it |

MCP tools use flat arguments:

- `product_register(id, repository, description?, local_path?, releases?)` creates
  a new ID only; existing IDs conflict, including archived IDs.
- `product_get(id)` reads the full record; `product_list(limit?, offset?, archived?)` returns paged compact
  summaries. Follow `next_offset` until null for all matching products. These reads do not touch the filesystem outside the ledger.
- `product_update(id, repository?, description?, local_path?, releases?)` patches
  only supplied fields. Explicit `local_path:null` clears placement and
  `releases:null` clears the policy; omitted fields remain unchanged.
- `product_archive(id)` is idempotent and retains existing tasks, record body,
  unknown frontmatter, and archive history. It prevents new runnable work.

Human HTTP has `GET /api/products`, `GET /api/products/{org}/{repo}`, create-only
`PUT /api/products/{org}/{repo}`, and partial `PATCH` at the same path. The worker read is
`GET /worker/products/{org}/{repo}`. Product policy does not authorize a particular
execution: its user's permission or restriction still applies. Unknown policy
must be resolved before publication, and `false` must not be treated as unknown.

Existing Markdown ledgers require no rewrite: keep the same `APP_DATA_DIR`.
Stop the old service, snapshot/back up the ledger, remove the obsolete projects
mount/configuration, and start the new version. Existing IDs, metadata, tasks,
archive history and unknown fields remain unchanged. Missing `local_path` and
`releases` remain unknown rather than being inferred from repository contents.
Set each active product's canonical repository and release policy through MCP
before execution. Placement is an optional hint; check that any local checkout
belongs to the canonical repository. This is an operator-owned migration of already registered records,
not a recurring discovery or registration process. SQLite imports still use
`bin/task-data import-sqlite` and retain original columns in `legacy`.

## Execution

`bin/task-loop --help` describes the standalone Python loop. It claims one ready task,
resolves a task worktree, starts a fresh `codex exec`, renews the lease, records the
result and appends a haystack note. The agent uses the installed development skills;
review and fixes are part of that delivery, not additional server-generated tasks.
Use `--once` for a single attempt and `--loop` for continuous execution.
`--execution-target sandbox|homeserver` selects the worker queue (default: sandbox).
This is independent of where task-server is hosted; each machine runs its own
loop/state directory and explicitly chooses its execution target.
The loop fetches fresh product metadata using its task's stable ID. If `local_path`
exists on this machine, it verifies that checkout's origin against `repository`.
Otherwise it clones the registered canonical repository into its own
`--state-dir/repositories/` directory and creates the normal per-task worktree.
It never derives placement from product ID, replicates another machine's absolute
path, or changes shared product metadata. A mismatched origin or archived product
fails before agent launch. It saves `product.json` beside `task.json` and includes
product metadata, execution target and release policy in the fresh agent prompt.

Agent failure, malformed output and timeout become blocked work with saved logs.
An expired lease is visible as interrupted work. Resuming a task retains its known
worktree and milestones, including dirty work created by an earlier attempt.
Unsent results are journaled and retried before another task is taken.

The loop does not grant release or deployment authority. Put the intended outcome
and relevant authorization in the task. It must report incomplete work as blocked,
even when an earlier implementation step succeeded.

For requests requiring development and real-machine checks, create a sandbox
development task and a homeserver deployment/check task whose single `depends_on`
is the development task ID. Development finishes on reproduction, fixes and
sandbox regression checks; pending real-machine work remains visible in the
successor rather than blocking completed development. Hand off the artifact/version,
application procedure, concrete operations, and observable success criteria (logs,
DB records, etc.). Ready homeserver work becomes claimable only after its dependency
is done. This split is explicit; releases do not automatically generate deployments.

## Execution handoff

MCP `task_checkpoint_get` takes `id`, optional `execution_id`, `limit`, and
`offset`. It returns `task_id`, `active_claim_id`, `checkpoints`, `total`, and
`next_offset` in oldest-first order. Each
checkpoint contains `execution_id` (the claim UUID), `revision`, `updated_at`,
and a free-form JSON object `values`. Without `execution_id`, executions are
paged (default 50, maximum 200), including expired ones; follow `next_offset`
until null. Use `execution_id` to directly select the current claim even when
it is beyond the first page. A task never claimed has an empty array.
A new claim starts with empty values at revision 0 and keeps older executions.
An existing lease from an older server is exposed at revision 0 without rewriting
its file on read; its first update persists the checkpoint.

MCP `task_checkpoint_update` takes:

```json
{
  "id": "task-id",
  "claim_id": "current-claim-uuid",
  "expected_revision": 0,
  "set": {"next_step": "wait_ci", "ci_url": "https://example.org/runs/123"},
  "delete_keys": ["obsolete_key"]
}
```

The MCP result is a receipt with the new revision; the worker HTTP result is
the updated checkpoint. `set` replaces only the specified top-level
keys (a nested object is one value); `null` is a value, not deletion. `delete_keys`
removes keys, including absent ones. Setting and deleting the same key is invalid.
Values are limited to 64 keys and 32768 bytes of serialized JSON per execution;
keys must be nonblank and at most 128 UTF-8 bytes. All values are JSON, and no
predefined handoff keys are required. Use references for large logs.

Updates require the live claim for that task and the exact current revision.
An expired/foreign claim or stale revision conflicts. Re-read after a conflict or
lost response and reapply only intended keys against the returned revision.
Each accepted patch increments revision and records its own update time. The
atomic task-file write leaves status, lease, commit, milestones and completion
reports unchanged. Checkpoint values are hints, never execution authority.

The equivalent trusted-network HTTP endpoint is `GET` or `POST`
`/worker/tasks/{id}/checkpoint`; GET accepts `?execution_id=...`, and POST has the
same arguments except `id` comes from the path. The browser task detail displays
values under **引き継ぎ情報**, labeled by current/previous execution and update time.

Do not store credentials or authentication tokens. Paths and agent handles can be
invalid on another machine or session: verify existence, repository/branch and
agent activity before reuse. On a new claim, inspect prior execution values and
copy only still-relevant keys explicitly. The loop reads this handoff before
starting its fresh agent, records machine/worktree/branch/log references, and
asks the agent to save its next step and evidence references at stage boundaries.
It reuses a saved dirty worktree only after matching the local machine and the
repository's registered worktree and branch. Its local journal still owns process
supervision and unsent report recovery; it is not the handoff source of truth.

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

For R2, install the AWS CLI and configure the upload settings below outside Git.
Then use `snapshot --upload` or `upload <archive>`.
Each generation has a unique timestamp; uploads do not synchronize deletions.
R2 uses its S3 endpoint and region `auto` ([Cloudflare documentation](https://developers.cloudflare.com/r2/examples/aws/aws-cli/)).
Without credentials, local snapshots and restore remain usable; no remote backup
is claimed. Configure retention on the backup destination to suit available space.

### Helper and deployment settings

These settings belong to the helper CLIs; the server does not read them.

| Variable | Required / default when unset | Purpose / invalid values | Reader |
| --- | --- | --- | --- |
| `KNOWLEDGE_REPO` | Optional / unset | Knowledge checkout passed into the fresh agent's context. The loop does not validate the path; empty or nonexistent paths do not fail the loop's startup. | [`bin/task-loop`](bin/task-loop) |
| `R2_ENDPOINT`, `R2_BUCKET` | Required for upload / no defaults | R2 endpoint and destination bucket. Missing or empty values fail upload before the AWS CLI is called; nonempty invalid values are left to the CLI to reject. | [`bin/task-data`](bin/task-data) |
| `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` | Required for upload / no defaults | Upload credentials passed to the AWS CLI. Missing or empty values fail before upload; invalid credentials fail at the provider. | [`bin/task-data`](bin/task-data) |
| `R2_PREFIX` | Optional / `task-server` | Archive key prefix. Leading/trailing `/` are stripped; empty or slash-only values place the archive at the bucket root. No further prefix validation is performed. | [`bin/task-data`](bin/task-data) |

For uploads, `PATH` must locate `aws`; the helper has no application default for
that OS search path and fails explicitly if the executable is missing. It maps
R2 credentials to `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`, sets
`AWS_DEFAULT_REGION=auto`, and sets both `AWS_REQUEST_CHECKSUM_CALCULATION` and
`AWS_RESPONSE_CHECKSUM_VALIDATION` to `when_required` in the child process.
These are generated AWS CLI settings, not additional server configuration.

`import-sqlite` takes its source database and new destination as positional
arguments. It does not read application storage settings. Loop options and local
snapshot/restore paths are CLI arguments; see each command's `--help`.

The [sandbox deployment](https://github.com/miyabisun/sandbox-server/blob/main/README.md#ローカル設定)
owns `TASK_SERVER_DATA_DIR` (host bind directory), `TASK_SERVER_UID` and
`TASK_SERVER_GID` (host file ownership), plus backup-wrapper settings.
The [home deployment](https://github.com/miyabisun/home-server/blob/main/docs/task-server.md)
owns its named volume and backup service. Both Compose files pass application
settings explicitly; a shared deployment env file is not the server's env file.
Deployment defaults and secret storage belong to those repositories.

## API

The browser uses `/api/tasks`, `/api/tasks/{id}`,
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

## Releases

Push a `vMAJOR.MINOR.PATCH` tag matching `Cargo.toml` and `Cargo.lock` to run
verification, build the container, smoke-test it with an empty ledger, and publish
the tested image as `ghcr.io/<owner>/<repo>:<version>` and `:latest`. The GitHub
Release records the image digest. Manual release runs must select a version tag.
PRs and manual CI runs perform verification; pushes to `main` do not prebuild or
publish images.

BuildKit retains intermediate stages at `ghcr.io/<owner>/<repo>:build-cache`
using the workflow's `GITHUB_TOKEN` with `packages: write`. This registry cache
works across release tags; no R2 credentials or separate cache service are needed.
An empty cache is valid. Cargo-chef separates dependencies from application code
and masks the application's version in its recipe, so a version bump can reuse
dependencies while the real manifest supplies the newly compiled application's
version. The frontend has an independent build stage.

The release profile uses Cargo's standard optimization and codegen settings,
with symbols stripped, to keep build time reasonable for this I/O-oriented server.
To run the same image check locally:

```sh
docker buildx build --load -t task-server:test .
bash .github/smoke-image.sh task-server:test
```

### Claim a ready task

Tasks own one `execution_target`: `sandbox` or `homeserver`. HTTP create/patch
(`/api/tasks`, `/api/tasks/:id`) and MCP `task_create`/`task_update` accept it;
detail/list responses expose it. `GET /api/tasks?execution_target=homeserver` and
MCP `task_list(execution_target: "homeserver")` filter by it. Omit the list filter
to see both destinations. The UI displays it on task rows/cards, edits it in the
create/edit form, and filters the common active list with a native select.

Existing task documents without the field read as `sandbox` without a bulk
rewrite. New tasks default to and persist `sandbox`. Updates that omit the field
preserve its value. Explicit null, empty, unknown and multiple values are invalid.
No generic labels or separate development/deployment field are added.

`POST /worker/claim` accepts a nonblank `worker`, optional `execution_target`
(default: `sandbox`), and optional nonblank `task_id` (one path segment excluding
`.` and `..`; null and non-string IDs are **400**):

```json
{"worker":"task-work:handoff", "execution_target":"homeserver", "task_id":"the-task-to-resume"}
```

The server atomically considers only tasks matching the execution target, even
with `task_id`. Omitting `task_id` selects eligible ready tasks by priority
(descending), then creation time (oldest first); ties retain ledger filename order.
An empty eligible queue, execution-target mismatch, or unfinished dependency
returns **204**, including ID selection. These conditions leave tasks waiting
without marking them blocked. ID selection never falls back to another task.
Old claim clients always use the sandbox queue and cannot take homeserver work.
Deploy the server contract before enabling homeserver clients.

Success is **200** with the existing `{claim_id, lease_expires_at, task}` envelope.
Selection, eligibility checks and lease creation share the ledger writer lock,
so concurrent claims on the same task have exactly one winner. The same claim
UUID controls heartbeat, checkpoint and report; no second ownership mechanism
is introduced.

Failures use the existing JSON `{code, error}` shape:

| HTTP | `code` | `error` for target selection |
| --- | --- | --- |
| 404 | `not_found` | `task_not_found` |
| 409 | `conflict` | `task_claimed` (existing claim; checked before readiness) |
| 409 | `conflict` | `task_not_active` (archived or historical control task) |
| 409 | `conflict` | `task_not_ready` |
| 409 | `conflict` | `dependency_missing` |
| 409 | `conflict` | `product_not_catalogued` or `product_archived` |
| 400 | `invalid` | Input validation message, including missing product metadata |

Target rejection does not block or reprioritize tasks. Existing accepted-report
recovery and expired-lease sweeping still run before selection: expired work
becomes blocked and must explicitly return to ready before being claimed again.
Queue mode retains its existing blocking of missing products/dependencies.
It skips records that still carry a claim ID, including hand-edited ready records,
so an existing claim is never overwritten.

For a bounded handoff, `task-work` consumers should send the authorized target as
`task_id`, check the returned task ID, and handle 400/404/409 without retrying an
unscoped claim. Update the consumer's queue instructions that previously said
ID selection was unavailable after deploying a server with this contract.
Callers may omit `task_id`; `bin/task-loop` explicitly sends its execution target;
`--once` still means one queue attempt, not selection of a particular ID.

### One original completion report

New workers submit one free-form Markdown original. Structured fields identify
what was done; the server never infers a verified milestone from the prose:

```json
{
  "claim_id": "the-current-lease",
  "outcome": "done",
  "report_markdown": "# Result\nImplemented the change.\n\n# Verification\ncargo test passed.\n\n# Remaining\nThe design idea is unverified.",
  "commit_sha": "the-subject-commit",
  "milestones": [{"name": "implemented"}, {"name": "verified"}],
  "checks": [{"name": "cargo test", "exit_code": 0}],
  "run": {"worker": "task-loop", "agent_exit": 0}
}
```

Send this to `POST /worker/report`. The response is the task object itself:
`report_id` is its numeric run ID, `report_ids` retains earlier report references,
and submitted milestones receive that same `report_id`. Milestone timestamps and
commit default to the report time and task commit. Evidence text is optional on
this path: it refers to the original instead of requiring another explanation.
Only explicitly submitted milestones are recorded. Checks are optional structured
command results; a `done` report cannot contain a nonzero exit code.

The original is the untruncated body of `runs/<id>.md`. Task instructions stay in
the task Markdown body. `GET /api/runs/<id>` and MCP `run_get` (`{"id":"42"}`)
return the original, task/claim IDs, subject commit and checks. The browser's
report links open the same record in execution history. Raw Markdown is displayed
as text, so embedded HTML cannot execute. Existing run notes, reading receipts,
task evidence and legacy reports remain readable; `summary`/`verification`
report requests still use the legacy path. Do not mix those fields into a new
`report_markdown` request.

The run is durably accepted before task state changes. Its `report_request`
metadata holds the replayable intent; it never contains another copy of the
Markdown original. On an interrupted task write, startup and subsequent task
operations/readers replay that accepted intent before lease expiry. A failure to
persist the original cannot complete a task. Resend the identical payload after a
lost response: the claim identifies its report and no new run or milestone is
created. A different payload for that claim conflicts. Later claims cannot be
rewound by an old resend. Task deletion retains the run original.

`bin/task-loop` sends its command/timing metadata in `run`, completes its journal
from the returned `report_id`, and does not make a second haystack request on
success. A refused/expired lease still uses `/worker/runs` to preserve the raw
body and logs without completing the task. Existing pending loop journals remain
readable. Knowledge selection consumes the same haystack originals through the
existing next/read receipt flow; its success is not a condition of completion.

## Compact MCP reads and mutation receipts

MCP and HTTP share the same domain operations and Markdown records. MCP returns
purpose-specific views; the existing browser and worker HTTP responses stay intact.
No ledger migration or rewriting is required.

| Tool | Response and explicit follow-up |
| --- | --- |
| `task_list(status?, product_id?, execution_target?, limit?, offset?)` | `tasks` with ID, product, execution target, title, status, priority, dependency/status and blocker; no prose or evidence. Default excludes closed tasks; supply one lifecycle status to include that status. |
| `task_get(id)` | Task body and current lifecycle, claim, commit, report ID, timestamps, transitions and run counts. It does not expand completion prose, milestones, history or checkpoint values. |
| `task_history(id, limit?, offset?)` | `entries` tagged by source field: `current_completion` (summary/verification/checks), `last_report`, `milestones`, `milestone_history`, `legacy_completion`, `report_ids`, `legacy`. Array entries retain their original index; historical values retain their original provenance, including overlapping legacy evidence. |
| `run_list(task_id?, product_id?, source?, unread?, limit?, offset?)` | `runs` with metadata and IDs, without original body/notes/tails/checks. `unread:true` selects unread; `false` selects read. |
| `run_get(id)` | Original run/report including Markdown body, notes and evidence; numeric run IDs are passed as strings. |
| `product_list(archived?, limit?, offset?)` | `products` with ID, description and archive metadata. `product_get(id)` reads repository, placement, release policy and the full original record. |
| `task_checkpoint_get(id, execution_id?, limit?, offset?)` | Explicit handoff values; see Execution handoff above. |

Every paged response includes `total` after filtering and `next_offset` (integer
or null). Limits are 1..200, default 50; offsets are nonnegative integers, default
0. Filters are applied before slicing. Task order is priority descending then ID;
products sort by ID, runs by numeric ID, checkpoints oldest first, and history by
the source-field order above then array index. An offset beyond the end returns
an empty page. These are views of current state, not frozen snapshots: restart a
traversal if concurrent edits change membership/order. `/worker/snapshot` remains
the consistent five-collection export for backup.

Task create/update/status responses contain `ok`, ID, compact task state,
`updated_at`, and `changed` field names; they do not echo submitted bodies or
historical evidence. Delete returns `ok`, ID and `deleted`. Product mutations
return current registered metadata without body or legacy fields. Checkpoint
updates return `ok`, `task_id`, `execution_id`, `revision`, and `updated_at`,
without echoing `values`; use the returned revision for the next update.

Tool-specific schemas reject unknown or irrelevant arguments. Invalid statuses,
product IDs, page sizes, types and negative offsets fail explicitly. A task patch
can clear `product_id`, `depends_on`, `release_level`, or `commit_sha` with null;
omitted fields are unchanged. Other optional task fields do not accept null.

Clients migrating from unpaged MCP reads must follow `next_offset` with the same
filters until null, fetch product metadata before acting on release policy, and
use the explicit history/run/checkpoint tools when those records are needed.
`bin/task-loop` uses the unchanged worker HTTP contract and needs no adapter.
