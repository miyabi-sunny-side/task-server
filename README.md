# Task Server

Task control plane. One Axum process serves the JSON API, two MCP endpoints, and
the compiled Svelte Task Card UI. A single sqlite database is the source of
truth: the transaction boundary is a sqlite transaction. Workers claim and
report over HTTP or MCP; the server issues the review and the merge each report
earns, so the one step left to a person is the release.

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
| POST | `/api/tasks` | create a task in `draft` (optional `release_level`: `patch` by default, `minor`, `major`), returns 201 |
| GET | `/api/tasks/{id}` | Task Card: the task plus `available_transitions`, `latest_review` and `runs_count` |
| PATCH | `/api/tasks/{id}` | edit `title`, `body`, `product_id`, `priority`, `branch`, `release_level` |
| DELETE | `/api/tasks/{id}` | remove a `cancelled`, `dropped` or `released` task with the review / merge / rework subtasks that named it; anything open is 409. Answers `{deleted: {id, subtasks}}` |
| GET | `/api/closed` | finished (`done`, `approved`, `merged`, `released`) and `cancelled` `normal` work in one list, most recently closed first; each row adds `closed_at` (the sort key) beside `done_at` |
| POST | `/api/tasks/{id}/status` | move the task to `{"status": "..."}`; `approved`, `merged`, and `released` are refused |
| POST | `/api/runs` | the rescue leaves a note on a task: `{task_id, note}`; the source is `rescue` whatever the body says. 201 with the row |
| GET | `/api/runs?since=<id>&limit=<n>&task_id=<id>` | the haystack from `id > since` upward, at most `limit` (default 100, max 500); `next` is the `since` of the following page while one exists |
| GET | `/api/control` | `{ mergeable, pending_merges, pending_releases, pending_reviews, unreviewed, releasable, stuck }`; each `pending_merges` and `pending_releases` row adds `verification`, the reason a blocked one stopped, and a release row its `release_level`; `stuck` rows are `{task_id, kind, status, since, reason}` with `reason` one of `unclaimed`, `lease-expired`, `no-subtask`, `subtask-unclaimed`, `blocked`, `release-stalled` (thresholds: `APP_STUCK_*_SECS`), oldest first within each reason |
| POST | `/api/reviews` | reconciliation: `{"task_id": "..."}` issues a review task by hand, returns 201 |
| POST | `/api/merges` | reconciliation: `{"task_id": "..."}` issues a merge task by hand, returns 201 |
| POST | `/api/releases` | reconciliation: `{"product_id": "..."}` issues the release task of a product whose landed work has none, returns 201 |
| GET | `/api/products` | product list |
| GET | `/api/products/{id}` | one product |
| PUT | `/api/products/{id}` | create or replace a product |
| POST | `/worker/claim` | lease the next ready task; optional `kinds` routes by role and optional `idempotency_key` makes an uncertain response retryable |
| POST | `/worker/claim/release` | `{"claim_id": "...", "reason": "..."}` hands a live claim back: the task returns to `ready` with the reason on `verification`; a claim that is not live is 409 `claim_not_live` |
| POST | `/worker/runs` | append one run to the haystack; idempotent on `(claim_id, attempt, source)` — 201 with the row when written, 200 with the row already there on a resend |
| POST | `/worker/report` | report a commit, and `checks`, against a lease; `"outcome": "blocked"` records why the work could not be finished |
| POST | `/worker/review-report` | answer a claimed review with a verdict and findings |
| POST | `/mcp` | MCP over Streamable HTTP: the catalogue and the task lifecycle |
| POST | `/worker/mcp` | MCP over Streamable HTTP: claim and report |

Reads need an `X-Auth-User` (or `Tailscale-User-Login`) from the ingress, and
the name it carries is taken as given: which clients reach this server at all
is settled in front of it. Mutations additionally need `X-CSRF-Token`, which is
the one thing a cross-site page cannot produce. `Origin` is not read. Worker
HTTP routes and both MCP endpoints deliberately add no application-layer
authentication; bind address and firewall decide who reaches them. Each worker
report still has to match its lease's `claim_id`. Tasks are never physically
deleted while it is open; discard one by moving it to `cancelled` or `dropped`, and
delete it (`DELETE /api/tasks/{id}`) once it is over — or let the retention sweep do it.

## A worker in curl

The worker side of this server is three routes, so the whole protocol fits in a
shell script. This is the reference for the wire, not a worker: it claims once,
does one task, reports, and exits. Everything a real worker adds — the poll
loop, the build, the git plumbing — is its own business, because this server
starts no subprocess. It decides; the worker runs.

The URL, worker name, and checkout root come from the environment. There is no
worker secret to distribute; reachability is restricted outside the process.

```sh
set -eu   # a refused call is the end of the run, never a step it walks past

: "${TASK_SERVER_URL:?}"   # where this server is reached
: "${WORKER_NAME:?}"       # this loop's name, recorded on the lease
: "${PROJECTS_ROOT:?}"     # where this worker keeps its clones

json='Content-Type: application/json'

# Every field read out of a body below has to be a non-empty string, and
# `jq -e` is not that test: it fails on null and on a missing key, but an empty
# string is a value, so it exits 0 and hands back nothing — an empty product_id
# would point step 4 at $PROJECTS_ROOT itself. The type test is what rejects
# ""; `-e` is kept for the body that carries no JSON value at all.
field() {   # field <json> <name> -> the value, or a non-zero exit
  printf '%s' "$1" | jq -er --arg f "$2" '
    .[$f] as $v
    | if ($v | type) == "string" and $v != "" then $v
      else "\($f) is not a non-empty string\n" | halt_error(1) end'
}

# 1. Claim. `normal` only: review tasks answer on /worker/review-report, so a
# claim that took every kind would report a review to the wrong route. The key
# names this logical attempt. If the response is lost, retry this exact body;
# do not mint another key and accidentally take a second task.
claim_key=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
claim_body=$(jq -nc \
  --arg worker "$WORKER_NAME" \
  --arg key "$claim_key" \
  '{worker: $worker, kinds: ["normal"], idempotency_key: $key}')
# `curl -f` turns a domain refusal into a non-zero exit, and because this is a
# plain assignment, `set -e` ends the reference run.
card=$(curl -fsS "$TASK_SERVER_URL/worker/claim" -H "$json" -d "$claim_body")

# 2. An empty queue is an answer, not an error. There is no long poll.
card_status=$(printf '%s' "$card" | jq -r '.status // ""')
if [ "$card_status" = "no-work" ]; then
  echo "nothing ready"
  exit 0
fi

# 3. Read the card. Each of these is a plain assignment, so a body that is not
# a claim ends the run here instead of carrying an empty value forward.
claim_id=$(field "$card" claim_id)
product=$(field "$card" product_id)
branch=$(field "$card" branch)
task_id=$(field "$card" id)
title=$(field "$card" title)
echo "$task_id: $title"   # for the log, not the wire

# 4. Do the work. The clone is this worker's to find: product_id is `org/repo`,
# so the checkout is $PROJECTS_ROOT/$product on the branch the card names.
repo="$PROJECTS_ROOT/$product"
git -C "$repo" switch -c "$branch" 2>/dev/null || git -C "$repo" switch "$branch"
# ... build, test and commit here ...
commit_sha=$(git -C "$repo" rev-parse HEAD)

# 5. Report against the lease. `outcome` may be omitted and defaults to "done";
# it is spelled out here because "blocked" is the other answer this route takes.
# The body is captured before it is read: behind a pipe into jq, curl's exit
# status is discarded and a 409 `claim_mismatch` would read as a finished task.
report=$(curl -fsS "$TASK_SERVER_URL/worker/report" -H "$json" \
  -d "$(jq -nc \
        --arg claim_id "$claim_id" \
        --arg commit_sha "$commit_sha" \
        '{claim_id: $claim_id,
          commit_sha: $commit_sha,
          verification: "cargo test",
          checks: [{name: "cargo test", exit_code: 0}],
          outcome: "done"}')")

# 6. A 200 is not the whole answer. The reply is the Task Card again, so read
# it back through the same guard: that is what proves the report landed on a
# task rather than on a body that merely answered 200.
reported_id=$(field "$report" id)
reported_status=$(field "$report" status)
echo "$reported_id -> $reported_status"
```

A claim answers with the whole Task Card, and three of its fields are what it
takes to start: `claim_id` is the lease every later call is made against, and
`product_id` and `branch` say what to work on and where to put it — a task with
no branch of its own is given `task/<id>` as the claim is granted. The script
reads three more, and none of them decides how the work is done: `status`
separates an empty queue from a card, and `id` and `title` go to the log so a
human can see which task this run took. Going the other way, the report hands
back `commit_sha`, the `verification` a human reads, and the `checks` that ran.
The other kinds answer elsewhere and are deliberately not repeated here: a
review is claimed with `{"kinds": ["review"]}` and answered with a verdict on
`/worker/review-report`, a rework with `{"kinds": ["rework"]}` and finished on
this same route with the commit it added, and a merge with
`{"kinds": ["instant:merge"]}`; all three
bodies are in
[Reviewing, merging and releasing](#reviewing-merging-and-releasing).

`idempotency_key` makes a transport-uncertain claim recoverable. Generate a
fresh key for one logical attempt and, until a definitive response arrives,
retry the same `worker`, `kinds`, and key. A successful retry returns the same
live task and `claim_id`; it never consumes another task. Only successful
claims leave receipts. `no-work` leaves none, so that same key may claim work
that becomes ready later. Reusing a key with another worker or set of kinds, or
after its lease expires, answers 409 with code
`claim_idempotency_conflict`. The key does not renew the lease.

### Where the clone is, the worker decides

A card names a `product_id` and a `branch` and says nothing else about any
filesystem, and that is the boundary. `product_id` is `org/repo` — an identity
that is portable between machines, and shaped like the tail of a path rather
than a path — and the server keeps no path for it: the `products` table carries
`id`, `repository`, `description`, `releases`, and the timestamps that record
when the row changed and when it was archived. No path is among them.

`APP_PROJECTS_DIR` is not the missing half of that path. It is where *this*
process walks to derive product identity, in the server's own filesystem
namespace. The scanner reads that tree and never writes to it. A worker on
another machine cannot use its paths, and the server cannot use the worker's.

In the split deployment — task-server on the long-lived host, disposable
workers elsewhere — leave `APP_PROJECTS_DIR` unset. The SQLite `products` table,
curated through the HTTP API, is the catalogue authority. Each worker's
`<root>/<org>/<repo>` tree is only a checkout cache and working set. The two
sides share `product_id`; they do not share a mount or keep a second server-side
clone merely to feed the scanner. A worker must preflight its own checkout and
either obtain it or report the task blocked.

When a deployment does choose a derived catalogue, the scanner accepts this
in-root bare-backed checkout:

```text
<APP_PROJECTS_DIR>/<org>/<repo>/
├── .git       # gitdir: ./.bare
└── .bare/
```

The `gitdir:` target stays inside the product root, so it passes the same
boundary checks as an ordinary checkout. With no working tree, `description` and
`releases` are read from the commit tree at `HEAD`; loose and packed objects both
work. That scanner support does not make a remote worker cache the split
deployment's catalogue authority.

### What the protocol leaves to the worker

- **There is no long poll.** A claim answers immediately, and an empty queue is
  `200 {"status":"no-work"}`. A loop has to poll, and to poll with jitter so
  that several workers do not wake in step.
- **An uncertain claim is retried, not abandoned.** A worker keeps its logical
  attempt's `idempotency_key` and exact claim payload until it receives a
  definitive answer. A fresh key would be a fresh attempt and could consume a
  second task. Omitting the key retains the legacy, non-idempotent behaviour.
- **There is no heartbeat.** The deadline is on the card: a claim answers with
  `claim_expires_at`, the instant the claim was granted plus `CLAIM_TTL_SECS`
  (3600 seconds by default), written as UTC `YYYY-MM-DDThh:mm:ssZ`. It is
  computed once, when the lease is granted, and no route extends it. Passing it
  refuses nothing by itself: `/worker/report` finds the task by `claim_id` and
  reads no deadline. What overrunning does is make the row claimable again, and
  a claim compares no worker name — so *any* later claim of it, this same worker
  included, mints a fresh `claim_id`, and from that moment the old lease matches
  no task and its report is refused with code `claim_mismatch`. Size the TTL
  against the longest task, or lose the report at the end of it. That is not the
  only refusal a late report meets: a task moved out from under the lease by
  hand — pressed `cancelled`, say — still matches its `claim_id`, and the report
  is refused as `invalid` for the status it now sits in. A renew or heartbeat
  route is the next protocol priority: it would let short leases recover crashed
  machines promptly without making long tasks race their deadline.
- **There is no nack.** `outcome: "blocked"` is not one: it records a stop, with
  the reason in `verification` and the evidence in `checks`. On the `normal`
  task above it leaves the task in `blocked` for a person to press back to
  `ready`. A blocked `instant:merge` does not come back that way — `ready` is
  refused on it, and the attempt is called off and reissued instead; see
  [Reviewing, merging and releasing](#reviewing-merging-and-releasing). Either
  way, a worker that only wants to put the work back down has nothing but the
  lease running out. A nack or release route follows renew in priority: it helps
  graceful shutdown and checkout preflight failures, but a disposable machine
  that has already vanished cannot call it.
- **A claim filters by kind, never by product.** `kinds` is the only filter the
  queue offers, so each role must name its work explicitly. A normal worker uses
  `["normal"]`; otherwise an unrestricted claim may take an `instant:merge`,
  which is ranked ahead of ordinary work. The filter is routing, not
  authorization. A worker can still be handed a product it has no clone of. Its
  choices are to obtain the checkout or report `outcome: "blocked"` with the
  reason. Returning a server path would not help: the clone is missing from the
  worker's machine either way.

## The product catalogue

The `products` table is the register of product identity. Anything that names a
product — a task, a merge, a release — means a row in it.

Filing work and curating the catalogue are two different moments, so
`POST /api/tasks` accepts a `product_id` the catalogue has never heard of and
the task starts in `draft` as usual. Promoting it is where identity is required:
`POST /api/tasks/{id}/status` with `ready` answers 409 when the product is not
catalogued, when its working copy is gone, or when the task has no `product_id`
at all, and the task stays where it was.

Where the remedy lies depends on where the catalogue comes from, and the two
refusals are deliberately different codes:

| Code | What it means | The remedy |
| --- | --- | --- |
| `product_not_catalogued` | No product is registered under that id. With a project tree configured, that means no clone sits at `<org>/<repo>`. | Correct the task's `product_id`, or put the clone in the tree and restart. With no tree configured, `PUT /api/products/{id}`. |
| `product_archived` | The product *is* registered, and its working copy left the tree. | Restore that one clone; the next walk clears the mark by itself. |
| `product_required` | The task names no product at all. | Set a `product_id`. |

With [a project tree configured](#derived-from-the-project-tree) the catalogue is
a derived value, so `PUT /api/products/{id}` is not the way back from either
refusal: a row typed in by hand is archived by the next walk unless the tree
agrees with it.

Refusals that come from the server's own domain carry a stable `code` next to
their human `error` message, so an automated client branches on the reason
rather than on the prose:

```json
{
  "error": "product 'org/repo' is not in the product catalogue, so task t-1 cannot become ready; correct the product_id, or register the product through the configured catalogue source: put a clone at org/repo and restart when APP_PROJECTS_DIR is set, otherwise use PUT /api/products/org/repo",
  "code": "product_not_catalogued"
}
```

The codes are `unauthorized`, `forbidden`, `not_found`, `claim_mismatch`,
`claim_idempotency_conflict`, `invalid`, `conflict`, `product_required`,
`product_not_catalogued`, `product_archived`, `frontmatter`, `io`, and `db`.
An unknown `/api/*` path
answers in the same shape, as a 404 with code `not_found`.

One kind of failure is outside that contract: a request body that is not valid
JSON is rejected by the web framework before any handler runs, so it comes back
as `400` with a plain-text explanation and no `code` to branch on.

### Derived from the project tree

Set `APP_PROJECTS_DIR` and the catalogue stops being a roster anyone maintains:
every start walks that directory two levels deep, reads `<org>/<repo>`, and
reconciles the table against what is actually on disk. A hand-kept roster drifts
— a clone is deleted and its product sits in the catalogue forever — and the
filesystem already knows the answer.

A directory two levels down becomes a product when it is a git repository with an
`origin` remote. "A git repository" is git's own test rather than "it has a
config": the git directory has to carry a `HEAD` that reads as a ref (or a
detached object name), an object store, and a `refs` directory. A stray
`.git/config` with a remote in it is not a clone, and minting a product from one
would give an id nobody can check out.

Every field comes from repository data already on disk, and no `git` binary is
ever run. Normal clones use their working tree, including uncommitted changes.
Bare repositories describe themselves from their committed tree, read from loose
or packed objects, and `releases` is read from the committed tree for every
repository: the tree at the tip of the **default branch**, which is what
`refs/remotes/origin/HEAD` names, or `HEAD` when there is no remote-tracking
default. A bare clone that only ever fetches keeps its local `main` at the commit
it was cloned from while `origin/main` follows, so asking `HEAD` there would
describe a commit nobody ships; a normal clone's working tree can be stale or
partial for the same reason. A tree the walk cannot read (no commit yet, a ref
that is missing, an object that will not open) does not turn `releases` off: the
value already in the catalogue is kept and a warning names the product.

| Field | Derived from |
| --- | --- |
| `id` | Where it sits locally, `<org>/<repo>`. Never the remote's owner: `sunny-side/5ch-viewer` keeps that id even though it pushes to `miyabisun`. |
| `repository` | The `origin` URL in `.git/config`, normalised to its browsable `https://` form. |
| `description` | The first non-empty line of working-tree `README.md`, or its blob at bare `HEAD`, without its leading `#`. Absent is empty. |
| `releases` | Whether `.github/workflows` is a tree at the tip of the default branch (`origin/HEAD`, else `HEAD`) — the same test for a normal clone, a worktree and a bare repository, never the working tree. Git cannot store an empty directory, so at least one tracked path must be below it. Nothing in the directory is interpreted. A tag is not evidence because a version can be cut by hand. Unreadable: the previous value is kept. Note that a repository derived from rust-svelte-template ships its release workflow on `workflow_dispatch` only, so its first release is dispatched by hand even though the flag is on. |

A worktree and a submodule keep `.git` as a file naming the real git directory,
and that pointer is followed — including a worktree's `commondir` — because git
keeps those directories inside the superproject, which is inside the tree.

The tree is also the boundary: every path the walk opens is resolved through its
symlinks and has to land under `APP_PROJECTS_DIR`. A `.git` that is a symlink out,
a `gitdir:` pointer that is absolute or climbs out with `..`, and a `commondir`
naming somewhere else are all skipped rather than followed. Working-tree
metadata that resolves out is absent. A bare repository whose refs, loose or
packed objects, or alternate object stores resolve out is skipped as
`outside_root`.

Each entry that is not a product is named in the startup log with its reason, and
the summary line counts them per reason: `not_a_repository`,
`incomplete_repository`, `outside_root`, `no_origin`, `invalid_id`, `symlink`.

### A product that left the disk is archived, never deleted

Deleting the row would strand every task that named it: `tasks.product_id` has no
foreign key, so a merged task would keep an id nothing answers for, and the
history stops resolving. So the row stays and is marked. `GET /api/products`
reports the mark as `"archived": true`, and the startup log names each product it
archived together with how many tasks still point at it.

What the mark changes is only the future. The tasks that already named the
product read exactly as before, and promoting one to `ready` is refused with code
`product_archived` — deliberately not `product_not_catalogued`, because the
remedy is different: the product *is* catalogued, its working copy is not there.
Restore the clone and the next walk clears the mark on its own; nobody re-enters
the product by hand. `PUT /api/products/{id}` may still correct an archived
product's description or remote, and leaves the mark alone: a correction is not a
claim that the directory came back.

Reconciling is not an upsert. A row matching the tree in all four fields is not
written at all, so `updated_at` means "this product changed" rather than "the
server restarted" — and a product that is already archived is not marked twice.
A walk that comes back empty archives nothing and warns instead, because a
missing mount looks exactly like every product having been deleted at once — and
it warns whether or not the catalogue already had rows, since a first start
pointed at the wrong path is the case nobody has a stale row to notice it by. A
root that cannot be read stops the startup.

One consequence of a derived catalogue is worth stating plainly: a row created
with `PUT /api/products/{id}` is archived by the next walk unless the tree agrees
with it. With a tree configured, the API is no longer where the catalogue is
decided.

Unset `APP_PROJECTS_DIR` and nothing is walked: the catalogue is whatever the API
put there. The `APP_PRODUCTS_SEED` roster this replaces is retired, and a start
that still sets it is refused rather than quietly ignoring the file.

## Reviewing, merging and releasing

The last three steps of a task are not buttons a human presses on the status
API; they are earned. Two of them are not pressed at all: **the server issues
the review and the merge itself**, so the one judgement left to a person is the
release.

When a worker reports a `normal` task `done`, the same transaction issues that
task's `review` task. Nobody files it, and there is no window in which finished
work sits unread and unnoticed: if the review cannot be issued, the report is
refused whole rather than leaving the work stranded. The review inherits the
target's product, branch and priority, and takes a snapshot of the commit the
work reported: that commit is the subject of the review, and it is what an
approval is later checked against. Only one open review may target a task, so a
second issue answers 409; a review that answered is over, so the next round is
issued as `review:<id>~2`, `~3`, and so on. A report that repeats one already on
the record issues nothing — the first one's review is still the review of that
commit.

`POST /api/reviews` still exists, but it is a **reconciliation** handle rather
than the normal path: use it for work that lost its reader, which
`GET /api/control` lists under `unreviewed`. That list is empty whenever the
automatic issuing is working.

The reviewer claims it — `POST /worker/claim` with `{"kinds": ["review"]}` takes
review tasks alone — and answers on the review's own route:

```jsonc
{
  "claim_id": "...",
  "subject_commit_sha": "abc1234",
  "verdict": "approve",          // or "request_changes"
  "findings": "read the diff, ran the tests"
}
```

Both verdicts are successes: a review that asks for changes did its job. No
checks are demanded — a reviewer's evidence is what they wrote, and it is kept
on the review either way. `request_changes` does **not** hand the task back to
`ready`: the same transaction issues a `rework` task on the work's branch (see
[Rework](#rework) below), and the target waits in `wip` with no lease while
that rework runs. The findings are readable on the rework's `body` and, as
before, on the target's own card under `latest_review`. `approve` moves the task to `approved` — the one way that
status is reached — after confirming, inside that transaction, that the task is
still waiting in `done` and still on the commit the review was issued for. An
approval that arrives after the work moved on is refused with code
`review_subject_changed`, `review_target_moved`, or `review_subject_mismatch`,
and writes nothing.

**That same transaction issues the merge.** An approval is the last judgement
before the main line, so the `instant:merge` task the work has earned is written
alongside the promotion rather than waited for: approval and merge issue land
together or not at all. Nothing is left for a person to press in between.

That route is the *only* way a review finishes. `POST /api/tasks/{id}/status`
and the MCP `task_set_status` tool both refuse `done` on a review task with 400
and code `invalid`, and `available_transitions` never offers it: a pressed
`done` would record no verdict, tell the target nothing, and free it for the
next review as though the reading had happened. Calling an *open* attempt off is
a different act and stays available — `blocked`, `cancelled`, `dropped` — and a
cancelled or dropped attempt does let the next review be issued.

Once a review has answered, it is the record of that verdict and nothing moves
it: every status is refused with 400 and code `invalid`, and
`available_transitions` comes back empty. `blocked` is the one that mattered —
the one-open-review index counts a `blocked` attempt as live, so a finished
review pushed back there would keep the next review of that task from being
issued, and from `blocked` it could be handed back to the queue, claimed, and
answered a second time over the verdict already given. A finished attempt never
stands in the way: the next review of the reworked commit is issued as usual.

### Rework

A `rework` task is another pass over finished work, on the branch it already
has. The control plane issues it in two places, and nobody files one by hand:

- a review that answers `request_changes` issues `rework:<id>` with
  `rework_reason: "review"` and the findings as its `body`;
- a merge whose **rebase conflicted** issues `rework:<id>` with
  `rework_reason: "merge-conflict"` and the worker's report of the conflict as
  its `body` (see [When a merge cannot be integrated](#when-a-merge-cannot-be-integrated)).

The rework inherits the target's product, branch, priority and release level,
names the target in `rework_target_task_id`, and starts in `ready`. A second
round is `rework:<id>~2`, `~3`, and so on, like a review's. While a rework is
open its target is `wip` **with the lease columns empty**: that is what keeps
the target out of every queue — a claim never takes a `wip` row without a
lease, a review is only issued for `done` work and a merge only for `approved`
work, and pressing `ready` or `done` on the target by hand is refused with 409
and code `rework_in_flight` until the rework is cancelled or dropped.

A fixer claims it like ordinary work — `POST /worker/claim` with
`{"kinds": ["rework"]}`, or with no `kinds` at all, which takes every kind — and
finishes it on `POST /worker/report` with the commit it added:

```jsonc
{ "claim_id": "...", "commit_sha": "def5678", "verification": "cargo test" }
```

What the `done` leads to follows from the reason, in the same transaction. Sent
back by a review, the new commit becomes the target's, the target is `done`
again (its `done_at` keeps its first value) and the next review is issued for
the new commit. Sent back by a conflicted merge, the rebased commit becomes the
target's, the target is `approved` again — a rebase is not a second reading —
and the next merge is issued for it. A `blocked` report on a rework moves the
rework and its target to `blocked` with the same reason, which is the one case
that does wait for a person. A pressed `done` on a rework is refused, like a
pressed `done` on a review: only a report names the commit.

Only the control plane writes an `instant:merge` task, and that is an invariant
of the domain rather than a rule of one transport: task registration files
ordinary work only, so `POST /api/tasks` refuses `"kind": "instant:merge"` with
400 and code `invalid`, and the MCP `task_create` tool has no `kind` argument at
all. A hand-made merge would be a merge with no target — claimed ahead of every
other task, impossible to report, and so a standing block on the queue.

A task becomes **mergeable** once a review approved it, with a branch and a
commit, and no live merge already targets it. `done` is not enough: nothing
reaches the main line unread. The merge task inherits the target's product,
branch, and commit, starts in `ready`, and is claimed ahead of ordinary work.
Only one live merge may target a task, so a second issue answers 409. Cancelling
or dropping the attempt frees the target again, and the retry is issued under
its own id: `merge:<id>` first, then `merge:<id>~2`, `~3`, and so on.

`POST /api/merges` is the reconciliation handle here, the counterpart of
`POST /api/reviews`: `GET /api/control` lists approved work with no merge under
`mergeable`, and that list is empty while the automatic issuing works.

### One merge at a time, per product

A merge rebases its branch onto the main line, so two merges of the same product
cannot run at once — the second would be rebasing onto a line the first has not
written yet. So a merge waits while another merge of the same product is `wip`
or `blocked`: one that is running, or one that stopped and is waiting for a
person. `done`, `cancelled`, and `dropped` release the rest.

**Which of a product's merges goes first is not promised.** Two merges that are
both `ready` do not wait on each other — if they did, each would see the other
and the product would never move. What keeps them from running together is the
claim itself: it takes one row in one transaction, and from that moment the row
is `wip` and the rest of the product waits on it. Different products run side by
side: one product's jam never holds another's.

`GET /api/control` reports the outstanding merges under `pending_merges`. The
order is stable — oldest first, ties broken by id — and it is only that: it
tells you what is outstanding, not which one goes out next.

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

A merge report that claims success is refused when it carries no checks, or any
non-zero `exit_code`, and changes nothing — including a repeat report against a
merge that already landed. When every check passed, the merge finishes and its
target moves to `merged` in the same transaction.

### When a merge cannot be integrated

A rebase that conflicts, or a check that came back red, is not an error to throw
away — it is a result to keep. The worker says so on the same route, with
`outcome`:

```jsonc
{
  "claim_id": "...",
  "commit_sha": "abc1234",
  "verification": "rebase onto main conflicts in src/task.rs",
  "checks": [{ "name": "git rebase", "exit_code": 1 }],
  "outcome": "blocked"
}
```

That is a **successful report of a failure**: it answers 200. What it does next
depends on *which* step failed, and the worker says which through the name of
the red check:

- **The rebase itself conflicted** — a check named `git rebase` with a non-zero
  `exit_code`. That is a job for the branch's author, not a judgement for a
  person, so the merge is `dropped` (with the reason on `verification` and the
  evidence on `checks`), the product's train moves on, and the same transaction
  issues `rework:<target>` with `rework_reason: "merge-conflict"` and the report
  in its body (see [Rework](#rework)). The rebased commit the rework reports is
  merged again as `merge:<target>~2`, without another review.
- **A check ran red after the rebase** — anything else. The merge moves to
  `blocked`, `verification` keeps the reason and `checks` keeps the evidence,
  and the target does not move — nothing landed, so nothing advances to
  `merged`. A blocked merge stops its product's train, and that is intended:
  everything behind it would be rebasing onto a main line that is still
  waiting for a person. `GET /api/control` carries the reason on the blocked
  merge's row in `pending_merges`, under `verification`, so a screen can name
  the jam from that one payload.

The check gate above applies to success only; a worker saying it was blocked
is reporting the red check, not claiming it as a pass. `outcome` defaults to
`"done"`, so a worker written before this field existed keeps working
unchanged. Re-sending the identical report is idempotent and writes nothing new
— a repeated conflict report finds its merge already dropped; sending a
*different* reason for a task that is already blocked answers 400.

**How a merge ended is its worker's report, not a press.** `done` and `blocked`
are outcomes, and both are refused on an `instant:merge` task with 400 and code
`invalid`, on `POST /api/tasks/{id}/status` and the MCP `task_set_status` tool
alike; neither appears in `available_transitions`. A pressed `blocked` would
stop the product's train with no reason and no checks written on the row. A
pressed `done` would skip the check gate and the target landing in one go,
leaving approved work that never merged and that neither reconciliation window
shows — `pending_merges` stops at `done`, and `mergeable` still sees a live
merge holding the target. `wip` stays pressable, because it is not an outcome:
it says the attempt is running, exactly as a claim does, and `cancelled` and
`dropped` still release it. Ordinary `normal` work is unaffected and still takes
`done` and `blocked` by hand.

**A blocked merge is called off and reissued, never restarted.** Press
`cancelled` or `dropped` on it — that releases the train and frees the target —
and the merge for that work is issued again as a fresh attempt,
`merge:<id>~2`. Pressing `ready` on it is refused with 400 and code `invalid`,
on `POST /api/tasks/{id}/status` and the MCP `task_set_status` tool alike, and
`available_transitions` on a blocked merge is exactly `["cancelled", "dropped"]`.
Handing that row back to a worker would restart an attempt that still carries
the last run's reason and checks, pinned to a commit whose main line has since
moved. (Ordinary `normal` work that is `blocked` is unaffected: it still returns
to `ready` by hand.)

Green checks are not the whole gate. The merge carries the commit it was issued
for, and landing it is confirmed against the task in that same transaction: the
task has to be still `approved` *and* still standing on that commit. Work that
was taken back, redone on another commit, and approved again is refused with 409
and code `merge_subject_changed`, and neither the merge nor the task moves —
`approved` on its own never says which commit was approved, and the old merge
read one nobody signed off. Cancel it, and the work becomes a merge candidate
again for the commit the review actually approved.

Landing is where the release is decided. For a product with `releases` set, the
merge that lands the work issues an `instant:release` task in the same
transaction — unless one is already open, in which case the report that ends it
gathers what landed meanwhile. The release ships every `merged` task of the
product that no live release carries yet, points each at itself
(`release_task_id`), and takes its `release_level` from the largest level among
them; every task carries a level (`patch` by default, `minor`, `major`) from the
moment it is filed, and its review and merge subtasks inherit it. A release
worker claims the task with `kinds: ["instant:release"]`, runs the bump at that
level, and reports `release_tag` (`v<major>.<minor>.<patch>`) with the commit the
tag points at and green checks; the report moves the release task, the work it
carried, and their finished subtasks to `released` under that tag. A `blocked`
report keeps the work merged with the reason on the release task; the attempt is
called off (`cancelled` or `dropped`) and `POST /api/releases` issues the next
one by hand, re-pointing the work at it. `GET /api/control` lists open releases
in `pending_releases`; `releasable` is the reconciliation window for landed work
no live release carries, empty while the issuing works.

A product that does not release ends at the landing: the work and its finished
subtasks go straight to `released` with no tag. An archived product releases
nothing, whatever its stored `releases` says: the flag is derived from a clone
that is no longer in the tree, and the workflows that would build the release run
from that clone. Its landed work stays `merged`, out of `releasable`, and putting
the clone back is the whole remedy — the next walk re-reads the flag.

Stop the service with <kbd>Ctrl</kbd>+<kbd>C</kbd>.

## MCP

The same control plane also speaks MCP, over Streamable HTTP, from the same
process and against the same sqlite. Both endpoints use the trusted network
boundary and have no application-layer authorization:

| Endpoint | Authorization | Tools |
| --- | --- | --- |
| `POST /mcp` | none at the application layer | `product_list`, `task_create`, `task_get`, `task_list`, `task_update`, `task_set_status`, `task_delete` |
| `POST /worker/mcp` | none at the application layer | `task_claim`, `task_report`, `task_review_report` |

Point clients at `http://127.0.0.1:3000/mcp` or `/worker/mcp`. The `initialize`
handshake hands back an `Mcp-Session-Id` the client carries on every later
request.

Ingress identity, `Origin`, and CSRF checks do not run on MCP. An obsolete
`Authorization` header is ignored so old and new clients can overlap during
rollout. Worker reports remain bound to their leases by `claim_id`.

Catalogue writes stay off MCP and are made over HTTP. Review, merge and release
issuance remains owned by the control plane; their HTTP routes are
reconciliation handles, not worker tools. The one delete — `task_delete`, the twin of
`DELETE /api/tasks/{id}` — takes only `cancelled`, `dropped` or `released` work. `task_create` therefore files ordinary work and takes no
`kind`, and `task_set_status` refuses `merged` and `released` with the same code
and for the same reason the HTTP status route does — one domain function answers
both, so neither transport can become a way around the other.

A refusal the domain owns is not a protocol failure, so it comes back as a tool
result with `isError: true`. Its `structuredContent` is the same
`{"error", "code"}` pair the HTTP API answers with, repeated verbatim in the
text content, so a client branches on the code rather than on the prose:

```json
{
  "isError": true,
  "structuredContent": {
    "error": "product 'org/repo' is not in the product catalogue, so task t-1 cannot become ready; correct the product_id, or register the product through the configured catalogue source: put a clone at org/repo and restart when APP_PROJECTS_DIR is set, otherwise use PUT /api/products/org/repo",
    "code": "product_not_catalogued"
  }
}
```

Two failures fall outside that shape. Arguments that do not deserialize also
answer `isError: true`, but with a plain-text explanation and no `code` to
branch on; an unknown method or tool name is a JSON-RPC error, not a result.

Neither endpoint reads the `Host` header. rmcp answers loopback authorities
alone by default — a guard against a page that re-resolves its own name to
`127.0.0.1` — and that default is switched off here, because a deployment is
reached through a reverse proxy under a name the proxy chooses, and the default
would refuse exactly that name. Deciding which names and which clients arrive is
the proxy's job.

Put both MCP endpoints behind an ingress you trust. Restrict them to the trusted
LAN or an equivalent network boundary; do not expose them directly to the
public Internet.

## Importing the markdown queue

Before sqlite the queue was a directory of markdown files with YAML
frontmatter — a live queue and an archive. `import-markdown` is the one
subcommand, and it reads them into the database; with no arguments the binary
is still the server.

```sh
APP_DB_PATH=data/task-server.db \
  cargo run --locked -- import-markdown --live queue --archive archive
```

Either directory alone is a valid import and omitting both is a usage error.
Each is read recursively for `*.md`, so an archive split into year directories
needs no flattening and no other layout is assumed. The task id is the file
stem: `queue/t-101.md` becomes task `t-101`, and so the same stem under both
directories is two files claiming one row rather than a merge. The database is
whichever one `APP_DB_PATH` names, created with its parent directory if it is
not there yet, so importing into a fresh file is a normal first run.

The markdown itself is only ever read. The import writes, moves, and deletes
nothing under either directory; removing the old queue afterwards is a separate
decision for whoever ran it.

The run is all or nothing. Every file is parsed and checked first, then a
single transaction writes the lot, so one unreadable file, one file with no
`title` or `status`, one duplicate id, one status nobody can map, or one row
already in the database with different content leaves the database exactly as it
was. Every file the run refused is listed at once rather than one per
attempt, and the exit code is non-zero:

```text
import refused (1 problem(s)); nothing was written
  queue/t-103.md: unknown v0.1 status 'frobnicated'
```

A run that goes through prints what it did:

```text
read 3 file(s): 2 live, 1 archive
inserted 3: wip 1, done 1, merged 1
skipped 0 (already imported, unchanged)
warning: 2 product(s) not in the catalogue: example/other, example/repo
```

Re-running the same input changes nothing: a file that would write the row
already sitting under its id is skipped, so the second run over those three
files reports `inserted 0` and `skipped 3`, and no row is rewritten. Editing a
file that was already imported is not an update — it is the conflict above, and
it stops the whole run.

The v0.1 statuses land in the v0.2 vocabulary like this:

| v0.1 status | imported as |
| --- | --- |
| `running` | `wip` |
| `awaiting_user` | `done` |
| `done` | `merged` |
| `release_requested` | `merged` |
| `release_failed` | `merged` |
| `draft`, `ready`, `released`, `blocked`, `cancelled`, `dropped` | unchanged |

`done` is the one that carries a decision rather than a rename: in v0.1 it meant
the human had accepted the work and it was over, which is `merged` here.
Importing it as `done` would raise every finished task of the old queue as a
merge candidate. A status neither the table nor the vocabulary knows refuses the
import instead of being quietly dropped.

Frontmatter that has a column of its own keeps it: `title`, `commit_sha`,
`verification`, `release_tag`, and the product, which is `target_space` or
`product_id` when there is no `target_space`. The product reaches its column
only when it reads as `org/repo`; a queue old enough to predate that convention
carries names like `tasks`, and refusing those would mean editing archived
history to migrate it. Such a row imports with no product, and the value it did
carry stays in the folded block below. Everything else is folded onto the
end of the body as one marked YAML block, led by the pre-mapping status, so
nothing that was written down is lost and the mapping can be read backwards:

````text
# wire up the health probe

Original body, kept verbatim.

---

## Imported v0.1 metadata

```yaml
status: running
area: development
tags:
- infra
```
````

Imported rows are ordinary work — `normal` kind, priority 0, no branch and no
claim — and their `created_at` and `updated_at` record when the import ran.

A product the imported rows name but the catalogue does not carry is a warning
and never a refusal: the import registers no product it was not given. The
catalogue gate is what asks for it later, so make sure the product is in the
catalogue — a clone in the project tree, or `PUT /api/products/{id}` where no
tree is configured — before promoting an imported task to `ready`. A row
that kept a pre-convention product reference is counted on its own line and
reaches the same gate, which refuses it with `product_required` until someone
decides which product it belongs to:

```text
read 3 file(s): 1 live, 2 archive
inserted 3: done 1, merged 1, released 1
skipped 0 (already imported, unchanged)
2 task(s) kept a legacy product reference in the body (product_id left unset)
warning: 1 product(s) not in the catalogue: example/repo
```

## Continuous backup

The database is the whole control plane, and it sits on one volume. Litestream
follows the write-ahead log and streams it to S3-compatible object storage, so
losing the volume costs about a second of writes instead of everything.

The server does none of this. It never starts Litestream and knows nothing
about a replica, so an image given no backup configuration comes up exactly as
it always did. Backup is a **sidecar**: a pinned `litestream/litestream` image
beside the server container, both mounting one **local named volume** so they
reach the same `/app/data/task-server.db`.

That arrangement has real limits, and they are not advisory:

- The volume must be a local one on the same Docker host. A network filesystem
  (NFS, SMB, GlusterFS) cannot carry sqlite's locks, and a Docker Desktop host
  bind mount can corrupt the write-ahead log.
- Both processes must run on the same kernel, so their file locks agree.
- Exactly one server and exactly one Litestream, never two of either.
- Both need write access to the volume; the image owns `/app/data` as
  `10001:10001`, so run the sidecar as `--user 10001:10001` too.

[`deploy/litestream.yml.example`](deploy/litestream.yml.example) is the config,
mounted at `/etc/litestream.yml`. Every value that names or opens the bucket is
an environment reference — `R2_ENDPOINT`, `R2_BUCKET`, `R2_PREFIX`,
`R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` — injected as secrets by whatever
runs the containers. Only `region: auto` and the database path are fixed. No
credential, bucket name, or endpoint belongs in this repository.

Writing the compose file, creating the bucket, and holding the secrets belong
to the deployment that runs the service, as its own change in its own
repository.

### Restoring

Restore is something a person does, never something startup does.

1. Stop the server and the sidecar.
2. Create an **empty new volume** — restoring over a live database is refused,
   and keeping the old one means you can still compare.
3. Run the same pinned Litestream image with the same config against it:
   `litestream restore -integrity-check full /app/data/task-server.db`.
4. Read the restored database before trusting it.
5. Swap the volume in, then start the **sidecar first and the server second**,
   so replication is watching before writes resume.

A backup that has never been restored is a guess.
[`deploy/restore-drill.sh`](deploy/restore-drill.sh) is the rehearsal: it
stands up a local MinIO as a stand-in for R2, replicates from a running server
while a task is created through the API, restores into a fresh volume with a
full integrity check, reads that task back out of the restored database, and
cleans up after itself. It uses public images and credentials it generates on
the spot; it never touches the real bucket.

```sh
deploy/restore-drill.sh
```

Docker is the only prerequisite. `TASK_SERVER_IMAGE`, `DRILL_PORT`, and
`DRILL_RESTORED_PORT` override the defaults.

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

## Releases

Continuous integration checks pushes to `main` and pull requests, and every push
to `main` also builds the container image and publishes it to
`ghcr.io/miyabi-sunny-side/task-server` as `sha-<short-sha>` and `edge` after
the image smoke test passed. Pull request runs build the same image but never
push it. The Dockerfile splits the Rust build with cargo-chef, so a change to
the sources alone reuses the cached dependency layer, and the workflows keep
that cache in GitHub Actions (`type=gha`).

A release is three steps in one run: **tag → image retag → GitHub Release**.
Pushing a SemVer tag (`bump-tag` does this) starts **Release container**, which
does not rebuild the image: it looks for the `sha-<short-sha>` image that CI
already pushed for the tagged commit and, when found, retags it as `<version>`
and `latest` with `docker buildx imagetools create` — no compilation happens
and the run ends in seconds. If that image is missing (CI on `main` never ran
for that commit, failed, or its cache expired), the workflow falls back to
building and pushing the image itself. The same run then creates the GitHub
Release for the tag with generated notes and one line naming the image it
published (`ghcr.io/miyabi-sunny-side/task-server:vX.Y.Z@sha256:…`), so a
person reading the Release knows what to deploy and a listener on release
notifications (version-server) can act on it. Re-running the workflow for a
tag that already has a Release leaves that Release as it is. `latest` keeps its
meaning — the newest release — which is what the sandbox's watchtower follows.
Only tag a commit after its CI checks pass; use a new SemVer tag for
corrections instead of moving an existing release tag.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `APP_DB_PATH` | `data/task-server.db` | SQLite database, for the server and for `import-markdown` alike. Created with its parent directory on first use. |
| `APP_BIND_ADDR` | `127.0.0.1:3000` | HTTP listener. Keep loopback unless you are sure you want otherwise. |
| `CLAIM_TTL_SECS` | `3600` | Lease lifetime for a claim. |
| `CALLED_OFF_RETENTION_DAYS` | `30` | Days a `cancelled` / `dropped` task stays before the startup sweep deletes it (one log line per task). `released` work is never swept. |
| `RUNS_RETENTION_DAYS` | `90` | Days a run keeps its `stdout_tail` / `stderr_tail` before the startup sweep blanks them. Every other field of a run is kept for good. |
| `APP_STUCK_UNCLAIMED_SECS` | `900` | How long a `ready` task (or an issued review / merge / release / rework) may wait unclaimed before `GET /api/control` lists it in `stuck`. |
| `APP_STUCK_SUBTASK_SECS` | `300` | How long `done` may go without a review, `approved` without a merge, or `merged` without a release before it is `stuck` (`no-subtask`). |
| `APP_STUCK_RELEASE_SECS` | `1800` | How long an `instant:release` may stay `wip` before it is `stuck` (`release-stalled`). |
| `APP_CSRF_TOKEN` | `dev-csrf` | Required on human mutation as `X-CSRF-Token`. |
| `APP_STATIC_DIR` | `client/dist` | Directory of the production frontend. |
| `APP_PROJECTS_DIR` | (unset) | Root of the `<org>/<repo>` project tree the catalogue is derived from at startup. Unset means nothing is walked and the catalogue is curated over the API alone; set and unreadable refuses the start. |
| `TASK_SERVER_ENV` | (unset) | Set to `production` to require `APP_CSRF_TOKEN` and drop the development identity. |
| `RUST_LOG` | `info` | `tracing-subscriber` filter, for example `task_server=debug,tower_http=debug`. |

With `TASK_SERVER_ENV=production` the process refuses to start unless
`APP_CSRF_TOKEN` is set. Which identities, origins, and hostnames may reach it
is decided by the ingress in front of it, which is the only thing positioned
to know.

### Network boundary

`APP_BIND_ADDR` only chooses an interface; it does not add encryption or
application-layer authentication. The MCP endpoints and worker HTTP routes
accept requests from any client that reaches them. Restrict those surfaces with
firewall or ingress policy to the trusted LAN or an equivalent network. Use TLS,
an authenticated VPN, or a tailnet when the path itself is not trusted. Never
expose the process directly to the public Internet.

A worker report is still scoped to the leased task by its `claim_id`. That
ownership check does not replace the network boundary: it rejects stale or
unknown leases, but it does not decide who may ask for new work.

The binary defaults to `127.0.0.1:3000`. The container image binds
`0.0.0.0:3000`; `EXPOSE` does not publish the port, so the container runtime's
port mapping, ingress, and firewall define who can reach it.

## Repository structure

```text
.
├── client/             # Svelte 5 Task Card UI
├── deploy/             # Litestream config template and the restore drill
├── src/                # Axum router, MCP endpoints, sqlite store, status machine
├── tests/              # Contract tests against the public API
├── Cargo.toml
├── DESIGN.md
└── rust-toolchain.toml
```

Unknown `/api/*` paths return the 404 JSON refusal. Other unknown paths fall
back to `client/dist/index.html` so the client router can restore a deep link.

## License

MIT. See [LICENSE](LICENSE).

## The haystack (`runs`)

Every run of an agent, every rescue note and every librarian watermark is one
row in `runs`, appended and never edited. A row is
`{id, at, source, worker, task_id, kind, claim_id, attempt, profile, model, skill_sha,
outcome, checks, commit_sha, diff_stat, agent_exit, agent_secs, stdout_tail, stderr_tail,
note, truncated}`; `source` is `worker`, `rescue` or `librarian`, and each source has its
own required fields (`worker`: `task_id`, `claim_id`, `outcome`; `rescue`: `task_id`,
`note`; `librarian`: `note`). The two tails and the note are cut at 8 KB and the row
says so with `truncated: true`.

Workers append over `POST /worker/runs` (the worker boundary), the rescue over
`POST /api/runs` (identity and CSRF), and a librarian reads forward from its
watermark with `GET /api/runs?since=`. Nothing is deleted; once a day's worth of
starts has passed `RUNS_RETENTION_DAYS`, the startup sweep blanks the two output
tails and keeps the rest. There is no index to maintain and no review to pass: the
haystack is the raw material, the summary lives in the knowledge repository.
