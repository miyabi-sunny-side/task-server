# task-server

Task control plane. One Axum process owns one sqlite database and serves the
JSON API, the MCP endpoints, and the compiled client.

## Invariants

- SQLite is truth. The transaction boundary is a sqlite transaction, taken
  `IMMEDIATE` so concurrent writers serialize at `BEGIN`.
- The database lives at `APP_DB_PATH` (default `data/task-server.db`).
  Migrations run at open and are keyed on `PRAGMA user_version`.
- A database on disk is in WAL or it does not open. `Db::open` reads
  `journal_mode` back and refuses anything else, because continuous backup
  follows the write-ahead log and a replica of a `delete` mode database would
  quietly stop tracking the truth. `busy_timeout` is read back the same way.
- What is exempt from that is decided from the path asked for, never from what
  sqlite reports afterwards: sqlite names a temporary database exactly as it
  names an in-memory one, so a blank `APP_DB_PATH` would otherwise open a
  scratch database that is neither replicated nor persisted. A blank or
  whitespace-only path is refused outright. Only the literal `:memory:` — and
  the internal in-memory open the tests use — is exempt; URI spellings such as
  `file::memory:` are treated as files and so fail the WAL read-back.
- The write-ahead log is left to sqlite. `wal_autocheckpoint` is never set and no
  checkpoint is ever forced: trimming the log is sqlite's business, and where the
  log is replicated the replicator has to have read a frame before it goes.
- Backup is a sidecar, never the server. The process replicates nothing and
  starts no Litestream, so an image given no backup configuration starts exactly
  as it always did. `deploy/litestream.yml.example` reaches every credential,
  bucket, and endpoint through the environment and carries none of them.
- HTTP is the only way in, carrying both the JSON API and the MCP endpoints.
  There is no file store and no git side effect.
- Discarding a task is a transition to `cancelled` or `dropped`, so the row
  stays auditable. The one physical delete is `DELETE /api/tasks/{id}` (and
  the MCP `task_delete`), allowed only for `cancelled`, `dropped` or
  `released` tasks — anything open answers 409 — and it takes the review,
  merge and rework subtasks that named the task with it, so no orphan points at
  a row that is gone. `runs.task_id` is text, not a key: the haystack keeps
  the task's runs. The startup sweep deletes `cancelled` / `dropped` tasks
  closed more than `CALLED_OFF_RETENTION_DAYS` (30) ago, one log line each;
  `released` is never swept. `closed_at` records when a task was called off.
- Product ids are `org/repo`, never a path. Task ids are one path segment.
- The `products` table is the register of product identity. A task may be
  created with a `product_id` that is not in it, but it may not be promoted:
  `ready` is refused with 409 and code `product_not_catalogued` when the product
  is uncatalogued, or `product_required` when the task has no `product_id`. With a
  project tree configured the remedy for `product_not_catalogued` is the tree (no
  clone sits at that id) or a corrected `product_id`, never a hand-written row. The
  gate fires on the transition only, so a row that is already `ready` or beyond
  is never demoted, and `available_transitions` still offers `ready`.
- `APP_PROJECTS_DIR` derives the catalogue from the `<org>/<repo>` tree on disk
  at startup and on `POST /api/products/rescan` (MCP `product_rescan`), in one
  transaction: a git repository with an `origin` remote is a
  product, and its id is the local placement rather than the remote's owner. A row
  matching the tree is not rewritten, so `updated_at` is not stamped by a restart.
  An empty walk changes nothing and warns — including on an empty catalogue, since
  the warning is about the walk; an unreadable root fails the startup. Unset means
  nothing is walked and the SQLite `products` table, curated over HTTP, is the
  catalogue authority. The retired `APP_PRODUCTS_SEED` refuses the start.
- In a split deployment the task-server host leaves `APP_PROJECTS_DIR` unset.
  Remote workers own `<root>/<org>/<repo>` checkout caches and working sets; the
  server owns catalogue identity in SQLite. Only `product_id` crosses that
  boundary. No server-side clone is maintained merely to mirror a worker cache.
- A git repository is git's own definition, not "it has a config": `HEAD` reading
  as a ref or an object name, an object store, and `refs`. Normal clones derive
  description and `releases` from the working-tree README and workflows directory,
  so uncommitted changes and an empty workflows directory count. Bare repositories
  derive them from the clean tree at `HEAD`; Git cannot store an empty directory.
  A tag is not release evidence.
- The walk never reads outside `APP_PROJECTS_DIR`. Every path is canonicalised and
  has to stay under the root: a `.git` symlink, a `gitdir:` pointer, or a
  `commondir` leading out is skipped as `outside_root` and counted. Worktree
  metadata resolving out is absent. A bare repository is skipped as `outside_root`
  if its refs, loose or packed objects, or alternate object stores resolve out.
  Worktree and submodule pointers are followed while they stay inside. This
  includes `.git` saying `gitdir: ./.bare` when `.bare/` stays inside the product.
- A product whose working copy left the tree is **archived, never deleted**:
  `products.archived_at` is set, the row stays so the tasks that named it keep
  resolving, and `ready` is refused with code `product_archived` — distinct from
  `product_not_catalogued`, because the remedy is restoring the clone. The next
  walk that finds the directory again clears the mark. `PUT /api/products/{id}`
  never sets or clears it.
- `task-server import-markdown --live <DIR> [--archive <DIR>]` is the only
  subcommand; no arguments is the server. Either directory alone is valid, both
  omitted is a usage error, and each is read recursively for `*.md` into
  `APP_DB_PATH`. The import is all or nothing: every file is parsed and checked
  first, then one transaction writes the lot, so an unreadable file, a missing
  `title` or `status`, a duplicate id, an unmappable status, or a row already
  in the database with different content writes nothing and exits non-zero with
  every refused file listed. The markdown is only read — the import never
  writes, moves, or deletes a file under either directory.
- A task id is the file stem, so the same stem in the live queue and the
  archive is two files claiming one row, not a merge. A file that would write
  the row already under its id is skipped, so re-running the same input leaves
  every row as it was.
- Every v0.1 status reaches the v0.2 vocabulary — `running → wip`,
  `awaiting_user → done`, `done`, `release_requested`, and `release_failed` →
  `merged`, everything else under its own name — and an unknown one refuses the
  import rather than being dropped. `done` is the decision, not a rename: v0.1
  `done` was accepted, finished work, and importing it as `done` would raise
  every finished task of the old queue as a merge candidate.
- Frontmatter the schema has no column for is folded onto the end of the body
  as one `## Imported v0.1 metadata` YAML block, led by the pre-mapping status,
  so nothing written down is lost and the mapping reads backwards. Imported rows
  are `normal` tasks at priority 0 with no branch and no claim. A product the
  import names that the catalogue lacks is a warning, never a refusal and never
  an auto-created row; the `ready` gate is what asks for it later.
- A product reference that does not read as `org/repo` is not a refusal either.
  It leaves `product_id` unset, keeps its value in the folded block, and is
  counted on its own summary line — a queue older than the convention migrates
  without rewriting its archive, and the `ready` gate decides the product later.
- Every refusal the domain owns — and every unknown `/api/*` path — answers
  `{"error": "<message>", "code": "<slug>"}`. The slug is stable and is what an
  automated client branches on; the message is not. A request body that is not
  JSON never reaches the domain: the axum extractor rejects it first, with 400
  and a plain-text explanation that carries no `code`.
- `approved`, `merged`, and `released` belong to the control plane. Every
  operator surface goes through `task::set_status_by_operator`, which refuses
  all three before delegating to `set_status`, so `POST /api/tasks/{id}/status`
  and the MCP `task_set_status` tool answer the same refusal from one place. The
  control plane keeps calling `set_status` directly, `available_transitions`
  never offers any of them, and the transition table still allows them because
  the control plane goes through it.
- `done` on a `review` task belongs to the control plane for the same reason.
  `task::set_status_by_operator` refuses it inside the transaction that reads the
  row, and `available_transitions` leaves it out, so neither HTTP nor MCP can
  finish a review without a verdict — which would also have freed the
  one-open-review index, whose predicate stops at `done` (and `released`). While the review is
  still open, only `done` is closed: `wip` is how a reviewer claims the task, and
  `blocked`, `cancelled`, and `dropped` stay pressable so an attempt can be
  called off. That `cancelled` and `dropped` release the index is the point of
  abandoning an attempt.
- A review that **answered** is terminal for every operator surface, and
  `available_transitions` on it is empty. `task::operator_refusal` treats a
  `review` task as answered when it is `done` *or* carries a `review_verdict` —
  `review_report` writes both in one statement, so the two always agree, and
  either mark alone is enough to say the attempt is over. Freezing it closes two
  holes: `blocked` would put a finished attempt back inside the one-open-review
  index and stand in the way of the next review, and from `blocked` the row would
  walk through `ready` to a claim and report a second verdict over the one the
  target lives by. Freezing strands nothing, because `done` frees the index
  whether or not the verdict is there.
- The transition table carries no `done → ready`. Work leaves `done` backwards
  only through a review that requested changes, which has its own atomic
  operation, so no human can reopen finished work by hand.
- Only the control plane issues `instant:merge`, `review`, and
  `instant:release`. `task::create` refuses every kind it owns, so `POST
  /api/tasks` answers 400 with code `invalid` and the MCP `task_create` tool has
  no `kind` argument to choose from. Those tasks are written by
  `task::issue_merge`, `task::issue_review`, and `task::ensure_release` alone,
  against targets they name; an orphan would be claimed, could never be
  reported, and would block the queue.
- **Reviews, merges, and releases are issued by the machine, not by a person.**
  A `done` report on a `normal` task issues that task's review in the same
  transaction, an approving verdict issues that task's merge in the same
  transaction, and a landing issues the product's release in the same
  transaction. `POST /api/reviews`, `POST /api/merges`, and `POST /api/releases`
  exist only as reconciliation handles for work that lost its next step;
  nothing in the ordinary flow calls them.
- Every task carries a `release_level` (`patch` by default, `minor`, `major`)
  from the moment it is filed, and its subtasks inherit it at issue. A release
  task's level is the largest among the work it ships, its id is
  `release:<newest target>`, and each shipped `normal` task points at it through
  `release_task_id`. A product carries at most one open (`ready`, `wip`,
  `blocked`) release; while one is open the next landing issues nothing, and the
  report that ends it calls `ensure_release` again to gather what landed.
- A `done` report on an `instant:release` task must carry `release_tag` matching
  `v<major>.<minor>.<patch>` and green `checks`; it moves the release task, the
  work pointing at it, and their finished subtasks to `released` under that tag
  in one transaction. A `blocked` report is kept like a blocked merge — reason on
  the row, work still `merged` — and the attempt is called off and reissued by
  hand, never restarted; `task::operator_refusal` treats a release exactly as it
  treats a merge. A product with `releases` unset ends at the landing: the work
  and its finished subtasks go to `released` with no tag, and no release is
  issued.
- A review is issued against a `normal` task that is `done` with a `branch` and a
  `commit_sha`. It inherits the target's product, branch and priority and
  snapshots the target's `commit_sha` as the subject of the review; its own
  completion never rewrites that. Findings live in the review's `verification`,
  the verdict in `review_verdict`, and no column duplicates either onto the
  target: `GET /api/tasks/{id}` derives `latest_review` from the review's own
  row.
- `POST /worker/report` with the default `outcome` finishes a `normal` task and
  calls `task::ensure_review` in the same transaction. `ensure_review` is a
  no-op when some review already holds the target — the test is the predicate of
  the one-open-review index, `status NOT IN ('done', 'cancelled', 'dropped', 'released')` —
  because that review either hands the work back or is refused as stale, and
  either way the work has a reader. Otherwise it issues one, and a review that
  cannot be issued takes the whole report down with it rather than leaving work
  finished and invisible to reviewers. The idempotent repeat of a report (same
  `commit_sha`, already `done`) issues nothing: the first one's review is still
  the review of this commit.
- `task::review_report` with `approve` calls `task::ensure_merge` in the same
  transaction that promoted the target to `approved`. `ensure_merge` is the
  mirror of `ensure_review` and skips on the merge index's own predicate,
  `status NOT IN ('cancelled', 'dropped')` — a landed merge keeps its target for
  ever, so `done` is *not* on that list. `request_changes` issues nothing.
- One open review per target, kept by a partial unique index whose predicate
  excludes `done`, `cancelled`, `dropped`, and `released` (since schema
  version 12; a target reviewed twice ships both rounds to `released`, and
  before that the release report collided on the index and rolled back). That
  is where it parts company
  with the merge index, which keeps `done`: a landed merge still owns its
  target, while a review that answered is over and must not block the next
  round. The two therefore need columns of their own. A retry is
  `review:<id>~2`, `~3`, …
- The attempt number is stored in `review_attempt`, and it — not the id, not the
  timestamp — is what `latest_review` orders by. Derived ids compare the wrong
  way round as text (`~9` after `~10`), and timestamps are whole seconds, so two
  attempts answered in the same second tie. Attempts of one target are strictly
  serial (the index means attempt *n+1* cannot exist while *n* is in flight), so
  the highest attempt is always the latest answer.
- `POST /worker/review-report` is the review's completion contract, and
  deliberately not `/worker/report`, which refuses a review task outright. It
  takes `{claim_id, subject_commit_sha, verdict, findings}`, runs no check gate,
  and treats `request_changes` as a success — the reviewer did their job and the
  answer is "not yet". `approve` promotes the target to `approved` in the same
  transaction after confirming the target is still `done`, still on the commit
  the review snapshotted, and that the body names that commit; the three
  refusals carry codes `review_target_moved`, `review_subject_changed`, and
  `review_subject_mismatch`, and write nothing. `request_changes` returns the
  target to `ready` in one write and deliberately skips the `ready` catalogue
  gate: this is the continuation of work already admitted, and an archived
  product would otherwise leave a task that can be neither approved nor handed
  back.
- A task is mergeable when it is `normal`, `approved`, carries a `branch` and a
  `commit_sha`, and no live merge already targets it. Issuing writes one
  `instant:merge` task per target, in `ready`, inheriting the target's
  `product_id`, `branch`, `priority`, and `commit_sha`. A partial unique index
  keeps that at one live merge per target; a second issue is 409. A cancelled or
  dropped attempt frees the target for a retry. `task::mergeable` is therefore a
  reconciliation window rather than a queue: it is empty whenever the automatic
  issuing works, and `POST /api/merges` is the handle for what is in it.
- A merge task is `merge:<target_id>`; a retry whose id is already taken appends
  `~2`, `~3`, … so the id stays deterministic and one path segment.
- **A product's merges run one at a time, and which one goes first is not
  promised.** Each merge rebases its branch onto the main line, so the second of
  a product would otherwise rebase onto a line the first has not written. A
  merge is claimable only while no *other* merge of the same `product_id` is
  `wip` or `blocked` — running, or stopped and waiting for a person. `ready`
  merges do **not** wait on each other: if they did, each would see the other
  and the product would never move again. Serialising them is the claim's own
  job, since it takes one row in one transaction and leaves it `wip`. `done`,
  `cancelled`, and `dropped` release the rest. The product is compared with
  `IS`, so two merges carrying no product are still each other's train, and the
  candidate is excluded from its own test by id, so a merge whose lease expired
  is retaken rather than overtaken.
- **No order is stored and none is guaranteed.** `task::pending_merges` lists
  oldest first with ties broken by id; that is a stable order for reading, not a
  distribution order. Nothing decides in advance which of a product's `ready`
  merges a claim will take.
- Nothing lands untested. A **successful** report on an `instant:merge` task is
  refused unless it carries `checks` and every `exit_code` is `0`, whatever the
  merge's current status — so the answer never depends on the order the reports
  arrived in. Accepting it moves the merge to `done` and its target from
  `approved` to `merged` in one transaction; a refusal changes neither row. The
  gate guards success only: a report that says it was **blocked** is reporting
  the red check, not claiming it as a pass, and is accepted with it (see below).
- **A merge that could not be integrated is written down, not rolled back.**
  `POST /worker/report` with `outcome: "blocked"` writes the reason to
  `verification` and the evidence to `checks_json`, and keeps `commit_sha`,
  because on a merge it is the subject the merge was issued for. Which step
  failed decides the rest, read from the checks (`task::is_rebase_conflict`),
  never from the free text: a red check named `git rebase` is a conflict, and
  the merge is `dropped` while the same transaction issues
  `rework:<target>` (`merge-conflict`) — the train moves on and the rebased
  commit is merged again as `merge:<target>~2` under the approval it has. Any
  other red check moves the merge to `blocked` for a person, and the target
  does not move — nothing landed. Re-sending the identical report is
  idempotent (a repeated conflict finds its merge dropped); a different reason
  on an already-blocked task is 400. `outcome` defaults to `done`, so a worker
  written before this keeps working.
- **A `rework` is issued by the control plane and finished by a report.** A
  review that answers `request_changes` and a merge whose rebase conflicted
  each issue `rework:<target>` (`~2`, `~3`, … per round) in their own
  transaction, carrying `rework_target_task_id`, `rework_reason` (`review` or
  `merge-conflict`), the target's branch, product, priority and release level,
  and the findings or the conflict report as `body`. While it is open the
  target is parked: `wip` with every lease column `NULL`, which is what keeps it
  out of every queue (`CLAIMABLE` needs a lease on a `wip` row, a review needs
  `done`, a merge needs `approved`), and `task::set_status_by_operator` refuses
  `ready` and `done` on it with code `rework_in_flight` until the rework is
  cancelled or dropped. A `done` report on the rework puts its commit on the
  target and hands the target back to the step it came from — `done` and the
  next review for `review`, `approved` and the next merge for `merge-conflict`
  — with `done_at` unchanged; a `blocked` report blocks the rework and its
  target with the same reason. `task::operator_refusal` refuses a pressed
  `done` on a rework, like on a review. A finished rework is carried to
  `released` with its target, like a review or a merge.
- **How a merge ended is the worker's report, never a press.** `done` and
  `blocked` are outcomes, and `task::operator_refusal` refuses both on an
  `instant:merge` task, so `POST /api/tasks/{id}/status` and the MCP
  `task_set_status` tool answer 400 with code `invalid` and neither status
  appears in `available_transitions`. A pressed `blocked` would stop the
  product's train with no reason and no checks on the row; a pressed `done`
  would skip the check gate and `land_merge_target` together, leaving the target
  in `approved` while the attempt reads as finished — out of `pending_merges`,
  which stops at `done`, and out of `mergeable`, which still sees a live merge
  holding the target. `wip` stays pressable: it says the attempt is running,
  which is what a claim says, and `cancelled`/`dropped` still release it. The
  transition table allows all of these, because the rule is about operators.
  Ordinary `normal` work is unaffected and still takes `done` and `blocked` by
  hand.
- **A blocked merge is called off and reissued, never restarted.** It stops its
  product's train, and the way out is `cancelled` or `dropped` followed by a new
  attempt — `merge:<target>~2` — not `ready` on the row that failed.
  `task::operator_refusal` refuses `blocked → ready` on an `instant:merge` task,
  so `POST /api/tasks/{id}/status` and the MCP `task_set_status` tool both
  answer 400 with code `invalid`, and `available_transitions` on such a row is
  exactly `["cancelled", "dropped"]`. The transition table still allows the edge,
  because the rule is about operators. Restarting the row would hand a worker an
  attempt still carrying the failed run's reason and checks, pinned to a commit
  whose main line has moved. Ordinary `normal` work that is `blocked` is
  unaffected and still goes back to `ready` by hand.
- Nothing lands unread either. `task::land_merge_target` confirms, in that same
  transaction, that the target is still `approved` **and** still on the commit
  the merge was issued for; a target that was reopened, redone and approved again
  on another commit is refused with code `merge_subject_changed`, and neither row
  moves. This is the review-side `review_subject_changed` guard applied to the
  other end of the same hazard: `approved` alone never says *which* commit was
  approved. The comparison uses the merge row as the report found it, because the
  report writes its own `commit_sha` over the snapshot.
- `task::unreviewed` and `task::mergeable` are alarms, not queues. A `done`
  report issues its own review and an approval issues its own merge, so both
  lists stay empty in a healthy control plane; anything in either is work that
  lost its next step — an attempt somebody cancelled, or a row written before
  the issuing was automatic — and it will sit there for ever, because neither
  `done` nor `approved` has a way forward except the step that went missing.
  Their whole job is to make that silence visible, and `POST /api/reviews` and
  `POST /api/merges` are the handles that clear them. `unreviewed` spells
  "live" exactly as the one-open-review index does, so a task is listed there
  precisely when a new review could be issued for it.
- A task may only reach `released` when its product has `releases` set and is not
  archived: the flag is derived from a clone that has left the tree, and the
  workflows that build the release run from that clone. The mark is read at the
  question rather than written over `releases`, so the stored flag stays and a
  restored clone releases again on the next walk. An archived product is left out
  of `releasable` too.
  `POST /api/releases` issues the release task of one product by hand. A
  product that does not release, one with a release already open, or one with
  nothing merged and uncarried, is 409.
- Claim hands out the next `ready` task, `instant:merge` first, then higher
  `priority`, then oldest — subject to the merge train, which is what keeps a
  product's merges strictly serial. The row is only taken while it is still
  `ready`, so two workers never hold the same task. An optional `kinds` narrows the
  candidates to the work one loop handles; empty or absent takes anything, so a
  loop written before roles existed keeps working. It is routing, not
  authorization. Every role should pass an explicit list; a normal worker passes
  `["normal"]` so merge work cannot win its claim.
- An optional non-blank `idempotency_key` identifies one logical claim attempt.
  The first successful claim and its lease receipt are written in the same
  `IMMEDIATE` transaction. Replaying the same worker and semantic `kinds` while
  that lease is live returns the same task and `claim_id`; it consumes no other
  task. A payload mismatch or expired lease is 409 with code
  `claim_idempotency_conflict`. `no-work` writes no receipt, and omitting the key
  retains legacy claim behaviour. The key never renews a lease.
- One task, one branch. A claim on a task without a `branch` sets
  `task/<id>`; an existing branch is never rewritten.
- A report is matched by `claim_id`. A stale or unknown `claim_id` is
  rejected with 409. Reporting the same `commit_sha` twice is idempotent.
- Clock is injectable. Default claim TTL is 3600 seconds (`CLAIM_TTL_SECS`).
  There is no renew or heartbeat route, so the TTL must exceed the longest task.
  Renew is the next protocol priority.
- A worker hands a live claim back with `POST /worker/claim/release`
  (`{claim_id, reason}`): the task returns to `ready`, the four lease columns
  are cleared, and `claim released by <worker>: <reason>` is appended to
  `verification`. Any kind; a review's `review_attempt` is not counted, because
  the attempt is over only when a verdict is written. A claim that is not live
  — expired, already reported, unknown — is 409 with code `claim_not_live` and
  writes nothing. It helps a graceful worker; it cannot recover a machine that
  already vanished, which is what the lease is still for.
- Listen on `127.0.0.1` by default (`APP_BIND_ADDR` may override). Binding a LAN
  interface adds reachability, not TLS. The container image binds
  `0.0.0.0:3000`; runtime port publishing and ingress remain the boundary.
- Worker HTTP routes and `/worker/mcp` have no application-layer authentication.
  Every client that reaches them may ask for work; `claim_id` and the lease bind
  later reports to the claimed task, not the caller to a worker identity.
  Restrict the worker surface with firewall or ingress policy to a trusted LAN
  or equivalent network. Use TLS, an authenticated VPN, or a tailnet when the
  path is not trusted, and never expose the process directly to the public
  Internet.
- Human identity comes from ingress (`X-Auth-User` or `Tailscale-User-Login`).
  The browser does not mint identity.
- Human mutation requires an ingress identity and `X-CSRF-Token`. The identity
  is taken at face value and `Origin` is not read: which clients reach this
  server is the reverse proxy's decision, and the token is what a cross-site
  page cannot produce. An obsolete `X-Worker-Capability` header is ignored and
  never substitutes for either requirement.
- MCP is a second transport, not a second domain. `/mcp` and `/worker/mcp` are
  Streamable HTTP endpoints in the same process, and every tool decodes its
  arguments and calls `src/task.rs` or `src/product.rs`. The transition table,
  the catalogue gate, and the SQL are never duplicated there.
- Neither `/mcp` nor `/worker/mcp` has an application-layer gate. An obsolete
  bearer is ignored during rollout; bind address, firewall, and trusted ingress
  decide which clients can reach either endpoint.
- Ingress identity, `Origin`, and CSRF are not applied to MCP. rmcp's
  loopback-only `Host` allowlist is switched off with `disable_allowed_hosts()`,
  because this server is reached through a reverse proxy that already decides
  which names it serves and the default would refuse the name that proxy
  forwards.
- `/mcp` carries `product_list`, `task_create`, `task_get`, `task_list`,
  `task_update`, and `task_set_status`; `/worker/mcp` carries `task_claim`,
  `task_report`, and `task_review_report`. Catalogue writes and releases remain
  HTTP-only; review and merge reconciliation handles remain HTTP-only control
  plane operations. `task_create` files ordinary work and takes no `kind`, and
  `task_set_status` refuses `approved`, `merged`, and `released` with code
  `invalid` through the same domain function the HTTP status route calls.
- A refusal the domain owns is not a protocol failure: it is a tool result with
  `isError: true` whose `structuredContent` is the same `{"error", "code"}` pair
  HTTP answers with, repeated in the text content. Arguments that fail to
  deserialize are also `isError: true` but carry text alone and no `code`; an
  unknown method or tool name is a JSON-RPC error.
- `TASK_SERVER_ENV=production` is fail-closed without `APP_CSRF_TOKEN`. MCP and
  worker reachability is configured at the bind, firewall, and ingress boundary
  rather than through a process secret.
- Unknown `/api/*` paths return the 404 JSON refusal, code `not_found`. Every
  other unknown path falls back to `client/dist/index.html` so the client router
  can restore a deep link.

## Status vocabulary

`draft → ready → wip → done → approved → merged → released`

`approved` is granted by an approving review report and by nothing else.

Sideways from any live status: `blocked`, `cancelled`, `dropped`.
`blocked` returns to `ready`. `wip` may fall back to `ready`. `released`,
`cancelled`, and `dropped` are terminal.

A `normal` task with an open `rework` on its branch is `wip` without a lease
and takes neither `ready` nor `done` by hand (code `rework_in_flight`); the
rework's report moves it. A `rework` task takes no pressed `done`.

On an `instant:merge` task an operator may press none of `done`, `blocked`, or
`blocked → ready`: how an attempt ended is its worker's report, and an attempt
that could not be integrated is called off (`cancelled` or `dropped`) and
reissued rather than restarted. What is left on a merge is `wip`, `cancelled`,
and `dropped`.

## API

| Method | Path | Authorization |
| --- | --- | --- |
| GET | `/healthz`, `/api/health` | none |
| GET | `/api/session` | read |
| GET | `/api/tasks`, `/api/tasks/{id}` | read |
| GET | `/api/done` | read |
| GET | `/api/closed` | read |
| POST | `/api/tasks` | human mutation |
| PATCH | `/api/tasks/{id}` | human mutation |
| DELETE | `/api/tasks/{id}` | human mutation |
| POST | `/api/tasks/{id}/status` | human mutation |
| GET | `/api/control` | read |
| POST | `/api/reviews`, `/api/merges`, `/api/releases` | human mutation (reconciliation handles) |
| GET | `/api/products`, `/api/products/{id}` | read |
| PUT | `/api/products/{id}` | human mutation |
| POST | `/api/products/rescan` | human mutation |
| POST | `/worker/claim`, `/worker/claim/release`, `/worker/report`, `/worker/review-report` | trusted network; no application auth |
| POST | `/mcp`, `/worker/mcp` | trusted network; no application auth |

`GET /api/tasks` returns summaries and hides `released` unless `?status=` asks
for a status explicitly; an unknown status is a 400. Single-task responses are
the full task plus `available_transitions`, and `latest_review` when a review
has answered for it.

`GET /api/closed` answers the closed screen directly (finished and cancelled
`normal` work by the moment it closed); `GET /api/done` stays for readers of the
older done list and answers it directly rather than composing several
`?status=` calls: every `normal` task whose status is `done`, `approved`,
`merged`, or `released`, newest-completed first (`done_at DESC, id DESC`).
`done_at` is a `tasks` column of its own — the moment a task first reached
`done` — because `updated_at` keeps moving through approval, landing, and
release and cannot answer "when did this finish". It is written once, on the
transition into `done` (by a worker report or an operator press), guarded by
`kind = 'normal'` so a `review` or `instant:merge` row never carries one, and
left alone by every later transition; a task sent back for rework and finished
again keeps its first `done_at`. A database that predates the column backfills
it from `updated_at` for every row already at `done` or past it — the best
recoverable estimate for history the column was not there to keep — and
leaves every other row `NULL` rather than inventing a completion that never
happened.

`POST /worker/runs` appends one row to the haystack (`runs`) over the worker
boundary; `POST /api/runs` is the same append for a person or the rescue (identity
+ CSRF, source forced to `rescue`); `GET /api/runs?since=&limit=&task_id=` pages
the haystack forward by `id` with a `next` cursor. Rows are appended and never
edited; the idempotency key of a worker's resend is `(claim_id, attempt, source)`;
each source has its own required fields; tails are cut at 8 KB (`truncated`);
the startup sweep blanks `stdout_tail` / `stderr_tail` past `RUNS_RETENTION_DAYS`
(90) and touches nothing else. The Task Card carries `runs_count`.

`GET /api/control` answers
`{ mergeable, pending_merges, pending_releases, pending_reviews, unreviewed,
releasable, stuck }`: the three `pending_*` lists are what the control plane has
in flight, and `mergeable`, `unreviewed`, `releasable`, and `stuck` are
reconciliation windows — all stay empty while the automatic issuing works. Each
`pending_releases` row is the summary plus `release_level` and `verification`.
`stuck` rows are `{task_id, kind, status, since, reason}`, one row per task,
`reason` one of `unclaimed`, `lease-expired`, `no-subtask`, `subtask-unclaimed`,
`blocked`, `release-stalled`, grouped in that order and oldest first inside a
reason. The thresholds are `APP_STUCK_UNCLAIMED_SECS` (900),
`APP_STUCK_SUBTASK_SECS` (300) and `APP_STUCK_RELEASE_SECS` (1800); the judgment
is the server's clock against them, never a reading of the list.

`pending_merges` is in a stable reading order — oldest first, ties broken by id
— and each row is the ordinary summary plus `verification`: the reason that merge stopped, or `null` while it
is running. That is what lets a screen name a jammed train from this one
payload, with no per-task request behind it. The MCP `task_set_status` tool and
`POST /api/tasks/{id}/status` both refuse `ready` on a `blocked` merge, so a
jam is cleared by calling the attempt off and letting the next one be issued —
and both refuse `done` and `blocked` on any merge, so neither window can be
emptied by a press instead of a report.

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
