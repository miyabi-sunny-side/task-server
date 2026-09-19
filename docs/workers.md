# Worker execution and handoff

[Setup and first task](../README.md) · [Operations](operations.md) · [API](api.md) · [Workers](workers.md)

## Execution

The optional [bin/task-loop](../bin/task-loop) requires Python 3, Git, and a configured
agent command (default: `codex exec`). Install the development skills used by that agent.
`bin/task-loop --help` describes its options. It claims one ready task,
resolves a task worktree, starts a fresh `codex exec`, renews the lease, records the
result and appends a haystack note. The agent uses the installed development skills;
review and fixes are part of that delivery, not additional server-generated tasks.
Use `--once` for a single attempt and `--loop` for continuous execution.
For example, from a source checkout with an authorized ready task in the `local` queue:

```sh
bin/task-loop --once --url http://127.0.0.1:3000 --execution-target local \
  --state-dir "$HOME/.local/state/task-loop"
```

This command launches the configured agent. The server alone does not execute tasks.
`--execution-target NAME` selects any externally configured worker queue. Omitting
it leaves selection to the server's external default; without one the claim fails.
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

For requests requiring development and real-machine checks, use the operator's
separate execution targets. A deployment/check task can use the development task ID
as its single `depends_on`. Development finishes on reproduction, fixes, and isolated regression checks.
Pending real-machine work remains visible in the successor task.
It does not block completed development. Hand off the artifact/version,
application procedure, concrete operations, and observable success criteria (logs,
DB records, etc.). Dependent ready work becomes claimable only after its dependency
is done. This split is explicit; releases do not automatically generate deployments.

## Claim a ready task

Tasks own one `execution_target` reference. HTTP create/patch
(`/api/tasks`, `/api/tasks/:id`) and MCP `task_create`/`task_update` accept it;
detail/list responses expose it. `GET /api/tasks?execution_target=field` and
MCP `task_list(execution_target: "field")` filter by exact name, including historical
names no longer configured. URL-encode names in HTTP queries. Omit the list filter
to see all destinations. The UI displays it on task rows/cards, edits it in the
create/edit form, and filters the common active list with a native select.

New assignments and claims accept only names from the external configuration.
Create/claim may omit the target only when an external default exists. Existing
task documents without the field use that default on reads; without one they are
shown as null/未設定. Reads never rewrite the stored record. Explicit historical
references survive removed choices and are marked 現在の設定にありません in the UI.
Read responses include `execution_target_configured` to distinguish those references
from current choices. Updates omitting the target preserve it, even when it is
unset or no longer configured. New assignments reject null, unconfigured or multiple
values. The single execution target is not a multi-label classification system.

`POST /worker/claim` accepts:

- A nonblank `worker`.
- An optional `execution_target`; omission uses the external default.
- An optional, nonblank `task_id`: one path segment, excluding `.` and `..`.
  Null and non-string IDs return **400**.

For example:

```json
{"worker":"task-work:handoff", "execution_target":"field", "task_id":"the-task-to-resume"}
```

The server atomically considers only tasks matching the execution target, even
with `task_id`. Omitting `task_id` selects eligible ready tasks by priority
(descending), then creation time (oldest first); ties retain ledger filename order.
An empty eligible queue, execution-target mismatch, or unfinished dependency
returns **204**, including ID selection. These conditions leave tasks waiting
without marking them blocked. ID selection never falls back to another task.
Old claim clients use only the externally supplied default. Changing that default
changes their queue, so preserve the deployment's current default during migration.
Missing defaults or unconfigured claim targets produce **400**, never another queue.

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

Target rejection does not block or reprioritize tasks. Accepted-report recovery and expired-lease sweeping run before selection.
Expired work becomes blocked. Return it explicitly to ready before claiming it again.
Queue mode retains its existing blocking of missing products/dependencies.
It skips records that still carry a claim ID, including hand-edited ready records,
so an existing claim is never overwritten.

For a bounded handoff, send the authorized task as `task_id` and check the returned ID.
Handle 400/404/409 without retrying an unscoped claim. 
Callers may omit `task_id`; `bin/task-loop` explicitly sends its execution target;
`--once` still means one queue attempt, not selection of a particular ID.

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

The equivalent HTTP endpoint is `/worker/tasks/{id}/checkpoint`.
GET accepts `?execution_id=...`. POST takes the same arguments, with `id` from the path. The browser task detail displays
values under **引き継ぎ情報**, labeled by current/previous execution and update time.

Do not store credentials or authentication tokens. Paths and agent handles can be
invalid on another machine or session: verify existence, repository/branch and
agent activity before reuse. On a new claim, inspect prior execution values and
copy only still-relevant keys explicitly. The loop reads this handoff before starting its fresh agent.
It records machine/worktree/branch/log references.
It asks the agent to save its next step and evidence at stage boundaries.
It reuses a saved dirty worktree only after matching the local machine and the
repository's registered worktree and branch. Its local journal still owns process
supervision and unsent report recovery; it is not the handoff source of truth.

## One original completion report

New workers submit one free-form Markdown original. Structured fields identify
what was done; the server never infers a verified milestone from the prose:

```json
{
  "claim_id": "the-current-lease",
  "outcome": "done",
  "report_markdown": "# Result\nImplemented the change.\n\n# Verification\ncargo test passed.\n\n# Remaining\nNo remaining work.",
  "commit_sha": "the-subject-commit",
  "milestones": [{"name": "implemented"}, {"name": "verified"}],
  "checks": [{"name": "cargo test", "exit_code": 0}],
  "run": {"worker": "task-loop", "agent_exit": 0}
}
```

Send this to `POST /worker/report`. The response is the task object itself.
`report_id` is its numeric run ID; `report_ids` retains earlier report references.
Submitted milestones receive the same `report_id`. Milestone timestamps and
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

`bin/task-loop` sends command/timing metadata in `run`.
It completes its journal from the returned `report_id`.
It does not make a second haystack request on success. A refused/expired lease still uses `/worker/runs` to preserve the raw
body and logs without completing the task. Existing pending loop journals remain
readable. Knowledge selection consumes the same haystack originals through the
existing next/read receipt flow; its success is not a condition of completion.

