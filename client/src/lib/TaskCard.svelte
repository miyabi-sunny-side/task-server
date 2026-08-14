<script lang="ts">
  import type { TaskCard as Task } from "./api";

  let {
    task,
    busy = false,
    error = "",
    onaction,
  }: {
    task: Task;
    busy?: boolean;
    error?: string;
    onaction?: (action: string, bump?: string) => void;
  } = $props();

  type ActionButton = { action: string; bump?: string; label: string };

  let buttons = $derived.by((): ActionButton[] => {
    const out: ActionButton[] = [];
    for (const action of task.available_actions) {
      if (action === "bump-tag") {
        for (const bump of ["patch", "minor", "major"] as const) {
          out.push({ action, bump, label: `bump-tag ${bump}` });
        }
      } else {
        out.push({ action, label: action });
      }
    }
    return out;
  });

  function isPrimary(button: ActionButton): boolean {
    return button.action === "done";
  }
</script>

<p class="meta">
  <span class="badge">{task.status}</span>
</p>
<p class="caption" data-field="commit_sha">
  <span class="caption-label">commit_sha</span>
  {task.commit_sha ?? ""}
</p>
<p class="caption" data-field="verification">
  <span class="caption-label">verification</span>
  {task.verification ?? ""}
</p>
<p class="body-text">{task.body}</p>
{#if error}
  <p class="state error">{error}</p>
{/if}
{#if buttons.length > 0}
  <div class="actions">
    {#each buttons as button (button.label)}
      <button
        class="btn"
        class:primary={isPrimary(button)}
        type="button"
        disabled={busy}
        onclick={() => onaction?.(button.action, button.bump)}
      >
        {button.label}
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

  .badge
    display: inline-block
    padding: var(--sp-1) var(--sp-2)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-full)
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .caption
    margin: 0 0 var(--sp-2)
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .caption-label
    margin-right: var(--sp-2)

  .body-text
    margin: var(--sp-3) 0 0
    white-space: pre-line

  .actions
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
    margin-top: var(--sp-4)
</style>
