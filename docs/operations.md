# Configuration, storage, and recovery

[Setup and first task](../README.md) · [Operations](operations.md) · [API](api.md) · [Workers](workers.md)

## Environment variables

Application settings are optional and are read at startup. Helper CLI
settings are listed separately under [Helper and deployment settings](#helper-and-deployment-settings).

| Variable | Default when unset | Purpose / invalid values | Reader |
| --- | --- | --- | --- |
| `APP_DATA_DIR` | `data/ledger` | Markdown ledger path, relative to the working directory. Missing directories are created. Blank paths, inaccessible storage or an already-locked ledger fail startup; an empty ledger beside a legacy `task-server.db` requires explicit import. | [`src/state.rs`](../src/state.rs), [`src/ledger.rs`](../src/ledger.rs) |
| `EXECUTION_TARGETS_FILE` | No seed | Read-only YAML that seeds execution targets once, only while the ledger has no definitions; see below. Then it is not read. Non-Unicode values always fail startup. While seeding, empty paths, unreadable files, invalid YAML/schema, names outside the label rule, duplicates or a default outside the list fail startup. | [`src/execution_target.rs`](../src/execution_target.rs) |
| `PORT` | `3000` | Decimal TCP port from `1` to `65535`. Empty, non-Unicode, signed, whitespace-padded, nonnumeric or out-of-range values fail startup with a `PORT` error. | [`src/port.rs`](../src/port.rs) |
| `LOG_LEVEL` | `info` | Logging verbosity: exactly `off`, `error`, `warn`, `info`, `debug`, or `trace`. Empty, non-Unicode or invalid values (including uppercase and module filters) use `info`. | [`src/logging.rs`](../src/logging.rs) |
| `CLAIM_TTL_SECS` | `3600` seconds | Integer from `1` through `86400`, inclusive. A claim and each heartbeat set expiry to now plus this lifetime. Empty, nonnumeric, whitespace-padded or out-of-range values fail startup. | [`src/state.rs`](../src/state.rs), [`src/task.rs`](../src/task.rs) |

Non-Unicode `APP_DATA_DIR` and `CLAIM_TTL_SECS` values are treated as unset.
The former `RUST_LOG` is ignored; use `LOG_LEVEL`. Build-time Cargo/CI variables
are not runtime application settings.

### Execution targets

Execution target labels name the queues where work runs. The labels and an
optional default live in the ledger at `settings/execution_targets.md`. Manage
them with `POST /api/execution-targets`, `DELETE /api/execution-targets/{label}`
or the matching MCP tools ([API](api.md#execution-targets)). Changes apply to task
creation, claims and the UI choices immediately, without a restart. Writes share
the ledger writer lock with other updates.

A label matches `^[a-z0-9][a-z0-9_-]*$`: lowercase ASCII letters, digits, `-` and
`_`, starting with a letter or digit. Quotes, `/`, `.`, spaces, uppercase and
non-ASCII characters are rejected. A leading `-` is rejected because it reads as a
command option. Names are matched exactly. Creating an existing label returns 409.
Deleting an unknown label returns 404, and deleting the current default returns 409.
The API cannot change the default or rename a label.

Deleting a label stops new assignments and claims to it. Existing task references
stay in their records and remain readable and filterable. The UI marks them
現在の設定にありません. The server never generates labels from task records and has
no built-in destination.

#### Seeding from a file

A new ledger has no definitions: no labels and no default. Creating the first
label through the API starts the definitions without a default. To start with a
default, seed the ledger on first start. Save a YAML file **outside this
repository** and point `EXECUTION_TARGETS_FILE` at it (relative paths use the
server's working directory):

```yaml
labels:
  - forge
  - field
  - lab_2
default: forge
```

`labels` is required and contains unique names that follow the label rule.
`default` is optional (or null); when present it must name one of the labels.
Unknown keys and wrong types are errors. An explicit empty list is valid.

At startup, if the ledger has no definitions and the variable is set, the server
imports the file once and fails to start if it is invalid. Once the ledger has
definitions, the file is not read, even when it changes or becomes invalid. This
includes definitions emptied through the API. The server never writes the file.

Containers receive the file through a read-only bind mount, with
`EXECUTION_TARGETS_FILE` pointing to its container path. The runtime UID/GID
`10001:10001` needs read access. Keep the existing data volume and `APP_DATA_DIR`.
When upgrading from a version that read the file at every start, keep the mount
and variable unchanged. The first start of the new version imports the current
labels and default. Removing the mount afterwards is optional. Keep the file for
rollback: an older version reads the file again and ignores labels created or
deleted through the API.

Without definitions the UI explains the absence and disables destination selection
for new tasks. A fetch failure offers retry without losing form text.
Existing tasks remain readable and other fields remain editable.

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
  idea/<idea-id>.md
  settings/execution_targets.md
```

Each document has YAML frontmatter and a Markdown body. IDs are encoded in filenames;
metadata retains the original ID. Unknown frontmatter fields survive server edits.
No persistent database or Git daemon is required. A Git repository can be initialized
here for ordinary diff/commit workflows; exclude `.lock` and temporary files.
The server does not automatically commit or push your task content.

Current status and milestones are separate.
Statuses are `draft`, `ready`, `wip`, `blocked`, `done`, `cancelled`, and `dropped`.
Milestones are `implemented`, `verified`, `reviewed`, `merged`, and `released`. Each milestone records a time, evidence and subject commit. Changed
commits require fresh applicable evidence; prior evidence remains history.
`done` means the requested task outcome is complete, not merely that code was written.

Reads reflect hand edits. Coordinate concurrent editing with execution, and stop
writers for bulk changes. Only one server opens a ledger directory at a time.
Individual record replacement is atomic; there is no general multi-file transaction.

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

Snapshots include tasks, ideas, catalogue, haystack, read receipts, execution
target definitions and migration history. Archives made before ideas or ledger
definitions existed restore with an empty `idea/` or `settings/` directory. A
server opening such a ledger seeds definitions from `EXECUTION_TARGETS_FILE`, if set.
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
| `KNOWLEDGE_REPO` | Optional / unset | Knowledge checkout passed into the fresh agent's context. The loop does not validate the path; empty or nonexistent paths do not fail the loop's startup. | [`bin/task-loop`](../bin/task-loop) |
| `R2_ENDPOINT`, `R2_BUCKET` | Required for upload / no defaults | R2 endpoint and destination bucket. Missing or empty values fail upload before the AWS CLI is called; nonempty invalid values are left to the CLI to reject. | [`bin/task-data`](../bin/task-data) |
| `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` | Required for upload / no defaults | Upload credentials passed to the AWS CLI. Missing or empty values fail before upload; invalid credentials fail at the provider. | [`bin/task-data`](../bin/task-data) |
| `R2_PREFIX` | Optional / `task-server` | Archive key prefix. Leading/trailing `/` are stripped; empty or slash-only values place the archive at the bucket root. No further prefix validation is performed. | [`bin/task-data`](../bin/task-data) |

For uploads, `PATH` must locate `aws`; the helper has no application default for
that OS search path and fails explicitly if the executable is missing. The child process receives these generated settings:

- R2 credentials as `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.
- `AWS_DEFAULT_REGION=auto`.
- `AWS_REQUEST_CHECKSUM_CALCULATION=when_required`.
- `AWS_RESPONSE_CHECKSUM_VALIDATION=when_required`.

These are generated AWS CLI settings, not additional server configuration.

`import-sqlite` takes its source database and new destination as positional
arguments. It does not read application storage settings. Loop options and local
snapshot/restore paths are CLI arguments; see each command's `--help`.

With a host bind mount, make the ledger writable by the container's UID/GID `10001:10001`.
The optional execution-target seed file needs read access and is mounted read-only.
The [Docker quickstart](../README.md#run-with-docker) uses a named data volume instead.
Deployment wrappers may use their own variable names; pass server settings explicitly.
Keep backup credentials outside the repository and container image.


