<script lang="ts">
  import type { TaskCard as Task } from "./api";
  import { blockedByLabel } from "./api";

  let {
    task,
    busy = false,
    error = "",
    ontransition,
    onedit,
  }: {
    task: Task;
    busy?: boolean;
    error?: string;
    ontransition?: (status: string) => void;
    onedit?: () => void;
  } = $props();

  let transitions = $derived(task.archived ? [] : task.available_transitions);
  // Every kind but `normal` wears its name beside the status. Naming the
  // kinds one by one would leave the next one silent.
  let kind = $derived(task.kind === "normal" ? "" : task.kind);
  // A review task's `commit_sha` is the commit it was issued to read, so it
  // is captioned for what it is rather than as the task's own output. One
  // caption, because the same sha printed twice reads as two commits.
  let commitField = $derived(
    task.kind === "review" ? "subject_commit_sha" : "commit_sha",
  );
  let review = $derived(task.latest_review);
  let reason = $derived(
    task.status === "blocked" ? task.verification || task.summary : null,
  );
</script>

<!-- The head reads in the same order as a list card (DESIGN.md, Detail
     page): product first, then the state. The title is the page's h1 above. -->
<div class="meta">
  <p class="product">{task.product_id}</p>
  <p class="caption">現在の状態</p>
  <p class="badges" data-field="current-status">
    <span class="badge">{task.status}</span>
    {#if task.archived}<span class="badge">履歴</span>{/if}
    {#if task.status === "blocked" && task.blocked_by}
      <span class="badge" data-blocked-by={task.blocked_by}
        >{blockedByLabel(task.blocked_by)}</span
      >
    {/if}
    {#if kind}
      <span class="badge">{kind}</span>
    {/if}
  </p>
</div>
<!-- The forest first: the one or two sentences a person reads as the
     completion report. The log (verification, checks) is folded below it,
     closed by default (DESIGN.md, Task Card). -->
{#if task.summary}
  <p class="summary" data-field="summary">{task.summary}</p>
{/if}
{#if reason}
  <section data-field="blocked-reason">
    <h2 class="caption">停止理由</h2>
    <p class="record-text">{reason}</p>
  </section>
{/if}
<section class="milestones" data-field="milestones">
  <h2 class="caption">到達実績</h2>
  {#each task.milestones ?? [] as milestone, index (index)}
    <div class="milestone">
      <p class="caption">
        <span class="badge">{milestone.name}</span> <time>{milestone.at}</time>
      </p>
      {#if milestone.commit_sha}<p class="caption">
          {milestone.commit_sha}
        </p>{/if}
      {#if milestone.evidence}<p class="record-text">
          {milestone.evidence}
        </p>{/if}
    </div>
  {:else}
    <p class="caption">到達実績はありません</p>
  {/each}
</section>
{#if task.milestone_history?.length}
  <details class="record" data-field="milestone-history">
    <summary class="caption record-head">過去の到達実績</summary>
    {#each task.milestone_history as milestone, index (index)}
      <p class="record-text">
        {milestone.name} · {milestone.at}
        {milestone.commit_sha ?? ""}
        {milestone.evidence ?? ""}
      </p>
    {/each}
  </details>
{/if}
<p class="caption" data-field={commitField}>
  <span class="caption-label">{commitField}</span>
  {task.commit_sha ?? ""}
</p>
<details class="record" data-field="verification">
  <summary class="caption record-head">作業記録</summary>
  <p class="record-text">{task.verification ?? ""}</p>
</details>
{#if task.checks && task.checks.length > 0}
  <details class="record" data-field="checks">
    <summary class="caption record-head">確認結果</summary>
    <ul class="checks">
      {#each task.checks as check (check.name)}
        <li class="record-text">{check.name}: exit {check.exit_code}</li>
      {/each}
    </ul>
  </details>
{/if}
<!-- A task that waits for another says so beside the other captions; a
     ready one whose dependency has not landed says, in one more muted line,
     that this is why no worker has it yet (DESIGN.md, Dependency). -->
{#if task.depends_on}
  <p class="caption" data-field="depends_on">
    <span class="caption-label">depends_on</span>
    <a class="dependency" href={`/tasks/${task.depends_on}`}
      >{task.depends_on}</a
    >
  </p>
  {#if task.status === "ready" && task.dependency_status}
    <p class="caption" data-field="waiting">
      waiting depends_on:
      <a class="dependency" href={`/tasks/${task.depends_on}`}
        >{task.depends_on}</a
      >
    </p>
  {/if}
{/if}
<!-- Above the body: a worker who was sent back to `ready` has to read the
     correction before the instruction it corrects (DESIGN.md, Review block).
     Neutral throughout — `request_changes` is a finished review, not a
     failure of this app, so the danger tokens stay out of it. -->
{#if review}
  <section class="review" data-field="latest_review">
    <p class="caption review-head">
      レビュー
      <span class="badge">{review.verdict}</span>
    </p>
    <!-- The verdict is read at once; the findings are the trees, folded. -->
    <details class="record" data-findings>
      <summary class="caption record-head">レビュー所見</summary>
      <p class="findings">{review.findings ?? ""}</p>
    </details>
  </section>
{/if}
<p class="body-text">{task.body}</p>
{#if error}
  <p class="state error">{error}</p>
{/if}
{#if transitions.length > 0 || (!task.archived && onedit)}
  <div class="actions">
    {#if !task.archived && onedit}<button
        class="btn"
        type="button"
        disabled={busy}
        onclick={onedit}>編集</button
      >{/if}
    {#each transitions as status (status)}
      <button
        class="btn"
        class:primary={status === "ready" && !busy}
        type="button"
        disabled={busy}
        onclick={() => ontransition?.(status)}
      >
        {status}
      </button>
    {/each}
  </div>
{/if}

<style lang="sass">
  .meta
    display: flex
    flex-direction: column
    gap: var(--sp-1)
    margin: 0 0 var(--sp-3)

  .product
    margin: 0
    font-size: var(--fs-sm)
    line-height: 1.4
    color: var(--c-on-surface)
    overflow-wrap: anywhere

  .badges
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-2)
    margin: 0

  .caption
    margin: 0 0 var(--sp-2)
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .caption-label
    margin-right: var(--sp-2)

  .summary
    margin: 0 0 var(--sp-3)
    font-size: var(--fs-md)
    line-height: 1.6
    color: var(--c-on-surface)
    overflow-wrap: anywhere

  // The folded log: a caption-toned summary line the reader opens on purpose.
  .record
    margin: 0 0 var(--sp-2)

  .record-head
    cursor: pointer
    margin: 0

  .record-text
    margin: var(--sp-1) 0 0
    font-size: var(--fs-sm)
    line-height: 1.5
    color: var(--c-on-surface)
    white-space: pre-line
    overflow-wrap: anywhere

  .checks
    margin: var(--sp-1) 0 0
    padding-left: var(--sp-4)

  .dependency
    margin-right: var(--sp-2)
    color: var(--c-link)

  // The card recipe, borrowed for a block inside the card.
  .review
    margin: var(--sp-3) 0 0
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)

  .review-head
    display: flex
    align-items: center
    gap: var(--sp-2)

  .findings
    margin: 0
    font-size: var(--fs-sm)
    line-height: 1.5
    color: var(--c-on-surface)
    white-space: pre-line
    // Findings quote shas, paths, and command lines; none of them may push
    // the narrow viewport sideways.
    overflow-wrap: anywhere

  .milestones
    margin: var(--sp-4) 0

  .milestone
    margin-bottom: var(--sp-3)

  .caption
    overflow-wrap: anywhere

  .body-text
    overflow-wrap: anywhere
    margin: var(--sp-3) 0 0
    white-space: pre-line

  .actions
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
    margin-top: var(--sp-4)
</style>
