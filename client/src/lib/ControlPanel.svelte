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
  import MergeTrains from "./MergeTrains.svelte";
  import Readout from "./Readout.svelte";
  import Reconciliation from "./Reconciliation.svelte";

  // The panel is no longer "the screen's controls": the server issues review
  // and merge on its own. It is the automated stretch of the pipeline plus the
  // one human decision that ends it — release — read top-down as what you can
  // do, then what is being carried for you (DESIGN.md, Control panel).
  let {
    fetchState,
    plane,
    busy = false,
    result = { kind: "none", message: "" },
    onrelease,
    onretry,
  }: {
    fetchState: "loading" | "error" | "ready";
    plane?: ControlPlane;
    busy?: boolean;
    result?: ActionResult;
    onrelease?: () => void;
    onretry?: () => void;
  } = $props();

  let releasable = $derived(plane?.releasable ?? []);
  let pendingReviews = $derived(plane?.pending_reviews ?? []);
  let pendingMerges = $derived(plane?.pending_merges ?? []);
  let unreviewed = $derived(plane?.unreviewed ?? []);
  let mergeable = $derived(plane?.mergeable ?? []);

  let carrying = $derived(
    releasable.length +
      pendingReviews.length +
      pendingMerges.length +
      unreviewed.length +
      mergeable.length,
  );

  // Empty still renders the release button with its reason: a control that
  // vanishes when idle teaches nothing about what would bring it back.
  let panelState = $derived(
    fetchState === "ready"
      ? carrying === 0
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
    <!-- The page's one accent marks the point where the pipeline stops and
         asks a person. A move the machine makes never takes it back. -->
    <div class="row" data-block="control">
      <button
        id="control-release"
        class="btn"
        class:primary={releasable.length > 0}
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
    <div class="result" data-result data-block="result" aria-live="polite">
      {#if busy}
        <p class="note">
          <span class="spinner" aria-hidden="true"></span>実行中…
        </p>
      {:else if result.kind === "success"}
        <p class="note">{result.message}</p>
      {:else if result.kind === "error"}
        <p class="error-banner" role="alert">{result.message}</p>
      {/if}
    </div>
    {#if pendingReviews.length > 0}
      <Readout
        name="reviews"
        block="reviews"
        caption="review 待ち"
        items={pendingReviews}
      />
    {/if}
    <MergeTrains pending={pendingMerges} />
    <Reconciliation {unreviewed} {mergeable} />
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
</style>
