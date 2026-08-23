<script lang="ts">
  import type { TaskCard as Task } from "./api";

  let {
    task,
    busy = false,
    error = "",
    ontransition,
  }: {
    task: Task;
    busy?: boolean;
    error?: string;
    ontransition?: (status: string) => void;
  } = $props();

  let transitions = $derived(task.available_transitions);
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
</script>

<p class="meta">
  <span class="badge">{task.status}</span>
  {#if kind}
    <span class="badge">{kind}</span>
  {/if}
</p>
<p class="caption" data-field={commitField}>
  <span class="caption-label">{commitField}</span>
  {task.commit_sha ?? ""}
</p>
<p class="caption" data-field="verification">
  <span class="caption-label">verification</span>
  {task.verification ?? ""}
</p>
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
    <p class="findings" data-findings>{review.findings ?? ""}</p>
  </section>
{/if}
<p class="body-text">{task.body}</p>
{#if error}
  <p class="state error">{error}</p>
{/if}
{#if transitions.length > 0}
  <div class="actions">
    {#each transitions as status (status)}
      <button
        class="btn"
        class:primary={status === "ready"}
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
    align-items: center
    gap: var(--sp-2)
    margin: 0 0 var(--sp-3)

  .caption
    margin: 0 0 var(--sp-2)
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .caption-label
    margin-right: var(--sp-2)

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

  .body-text
    margin: var(--sp-3) 0 0
    white-space: pre-line

  .actions
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
    margin-top: var(--sp-4)
</style>
