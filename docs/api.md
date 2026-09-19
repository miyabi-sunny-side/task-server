# HTTP and MCP reference

[Setup and first task](../README.md) · [Operations](operations.md) · [API](api.md) · [Workers](workers.md)

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

Register each product once before making its tasks ready. For example:

```sh
curl --fail-with-body -X PUT http://127.0.0.1:3000/api/products/example/project \
  -H 'Content-Type: application/json' \
  -d '{"repository":"https://github.com/example/project","releases":false}'
```

Replace the ID and repository with your project's values.
`releases:false` means this product must not publish releases.
Registration is create-only; use PATCH to update an existing product.
Assign it to a draft task before making that task ready:

```sh
curl --fail-with-body -X PATCH http://127.0.0.1:3000/api/tasks/first-task \
  -H 'Content-Type: application/json' -d '{"product_id":"example/project"}'
curl --fail-with-body -X POST http://127.0.0.1:3000/api/tasks/first-task/status \
  -H 'Content-Type: application/json' -d '{"status":"ready"}'
```

The browser can also edit task fields and status.
Setting `ready` makes a task available to workers; it does not start execution by itself.

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

## Compact MCP reads and mutation receipts

MCP and HTTP share the same domain operations and Markdown records. MCP returns
purpose-specific views; the existing browser and worker HTTP responses stay intact.
No ledger migration or rewriting is required.

| Tool | Response and explicit follow-up |
| --- | --- |
| `execution_targets_get()` | External execution names and optional default; no mutation or ledger definitions. |
| `task_list(status?, product_id?, execution_target?, limit?, offset?)` | `tasks` with ID, product, execution target, title, status, priority, dependency/status and blocker; no prose or evidence. Default excludes closed tasks; supply one lifecycle status to include that status. |
| `task_get(id)` | Task body and current lifecycle, claim, commit, report ID, timestamps, transitions and run counts. It does not expand completion prose, milestones, history or checkpoint values. |
| `task_history(id, limit?, offset?)` | `entries` tagged by source field: `current_completion` (summary/verification/checks), `last_report`, `milestones`, `milestone_history`, `legacy_completion`, `report_ids`, `legacy`. Array entries retain their original index; historical values retain their original provenance, including overlapping legacy evidence. |
| `run_list(task_id?, product_id?, source?, unread?, limit?, offset?)` | `runs` with metadata and IDs, without original body/notes/tails/checks. `unread:true` selects unread; `false` selects read. |
| `run_get(id)` | Original run/report including Markdown body, notes and evidence; numeric run IDs are passed as strings. |
| `product_list(archived?, limit?, offset?)` | `products` with ID, description and archive metadata. `product_get(id)` reads repository, placement, release policy and the full original record. |
| `task_checkpoint_get(id, execution_id?, limit?, offset?)` | Explicit handoff values; see [Execution handoff](workers.md#execution-handoff). |

Every paged response includes `total` after filtering and `next_offset` (integer
or null). Limits are 1..200, default 50; offsets are nonnegative integers, default
0. Filters are applied before slicing. Tasks sort by priority descending, then ID. Products sort by ID; runs sort by numeric ID.
Checkpoints sort oldest first. History uses the source-field order above, then array index. An offset beyond the end returns
an empty page. These are views of current state, not frozen snapshots: restart a
traversal if concurrent edits change membership/order. `/worker/snapshot` remains
the consistent five-collection export for backup.

Task create/update/status responses include compact task state and the ID.
They also include `ok`, `updated_at`, and `changed` field names.
They do not echo submitted bodies or historical evidence. Delete returns `ok`, ID and `deleted`. Product mutations
return current registered metadata without body or legacy fields. Checkpoint
updates return `ok`, `task_id`, `execution_id`, `revision`, and `updated_at`,
without echoing `values`; use the returned revision for the next update.

Tool-specific schemas reject unknown or irrelevant arguments. Invalid statuses,
product IDs, page sizes, types and negative offsets fail explicitly. A task patch
can clear `product_id`, `depends_on`, `release_level`, or `commit_sha` with null;
omitted fields are unchanged. Other optional task fields do not accept null.

For clients migrating from unpaged MCP reads:

- Follow `next_offset` with the same filters until null.
- Fetch product metadata before acting on release policy.
- Use the explicit history/run/checkpoint tools to retrieve those records.

`bin/task-loop` uses the unchanged worker HTTP contract and needs no adapter.

