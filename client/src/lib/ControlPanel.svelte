<script lang="ts" module>
  // The outcome of the last control action, reported in place and never
  // auto-dismissed (DESIGN.md, Result line).
  export interface ActionResult {
    kind: "none" | "success" | "error";
    message: string;
  }
</script>

<script lang="ts">
  import type { ControlPlane } from "./api";

  let {
    fetchState,
    plane,
    busy = "",
    result = { kind: "none", message: "" },
    onmerge,
    onrelease,
    onretry,
  }: {
    fetchState: "loading" | "error" | "ready";
    plane?: ControlPlane;
    busy?: "" | "merge" | "release";
    result?: ActionResult;
    onmerge?: () => void;
    onrelease?: () => void;
    onretry?: () => void;
  } = $props();

  let mergeable = $derived(plane?.mergeable ?? []);
  let pending = $derived(plane?.pending_merges ?? []);
  let releasable = $derived(plane?.releasable ?? []);

  // Empty still renders both buttons with their reason: a control that
  // vanishes when idle teaches nothing (DESIGN.md, Control panel).
  let panelState = $derived(
    fetchState === "ready"
      ? mergeable.length === 0 &&
        pending.length === 0 &&
        releasable.length === 0
        ? "empty"
        : "success"
      : fetchState,
  );
</script>

<section class="panel" data-region="control" data-state={panelState}>
  {#if panelState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if panelState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => onretry?.()}>
        再試行
      </button>
    </div>
  {:else}
    <div class="row">
      <button
        id="control-merge"
        class="btn"
        class:primary={mergeable.length > 0}
        type="button"
        aria-disabled={mergeable.length === 0 ? "true" : undefined}
        aria-describedby="merge-note"
        disabled={busy === "merge"}
        onclick={() => mergeable.length > 0 && onmerge?.()}
      >
        merge
      </button>
      {#if mergeable.length > 0}
        <span class="pill" id="merge-note" data-count>{mergeable.length}</span>
      {:else}
        <span class="note" id="merge-note">merge 可能な task はありません</span>
      {/if}
    </div>
    <div class="row">
      <button
        id="control-release"
        class="btn"
        type="button"
        aria-disabled={releasable.length === 0 ? "true" : undefined}
        aria-describedby="release-note"
        onclick={() => releasable.length > 0 && onrelease?.()}
      >
        release
      </button>
      {#if releasable.length > 0}
        <span class="pill" id="release-note" data-count>
          {releasable.length}
        </span>
      {:else}
        <span class="note" id="release-note">
          release 可能な product はありません
        </span>
      {/if}
    </div>
    <div class="result" data-result aria-live="polite">
      {#if busy !== ""}
        <p class="note">
          <span class="spinner" aria-hidden="true"></span>実行中…
        </p>
      {:else if result.kind === "success"}
        <p class="note">{result.message}</p>
      {:else if result.kind === "error"}
        <p class="error-banner" role="alert">{result.message}</p>
      {/if}
    </div>
    {#if pending.length > 0}
      <div class="pending">
        <p class="note">merge 進行中</p>
        <ul class="cards">
          {#each pending as item (item.id)}
            <li>
              <a class="card" href={`/tasks/${item.id}`}>
                <span class="name">{item.title}</span>
                <span class="product">{item.product_id}</span>
              </a>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  {/if}
</section>

<style lang="sass">
  .panel
    display: flex
    flex-direction: column
    gap: var(--sp-3)
    margin-bottom: var(--sp-5)
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)

  .row
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-3)

  .pending
    display: flex
    flex-direction: column
    gap: var(--sp-2)
</style>
