<script lang="ts">
  import type { ControlPlane } from "./api";
  import MergeTrains from "./MergeTrains.svelte";
  import Readout from "./Readout.svelte";
  import Reconciliation from "./Reconciliation.svelte";
  import Releases from "./Releases.svelte";

  // The panel is what the server is carrying, read top-down in pipeline
  // order: reviews waiting, the merge trains, the releases, then anything the
  // automation failed to carry. Review, merge and release are all issued by
  // the server, so the panel holds no control and asks the human for nothing;
  // the top page has no primary button (DESIGN.md, Control panel).
  let {
    fetchState,
    plane,
    onretry,
  }: {
    fetchState: "loading" | "error" | "ready";
    plane?: ControlPlane;
    onretry?: () => void;
  } = $props();

  let pendingReviews = $derived(plane?.pending_reviews ?? []);
  let pendingMerges = $derived(plane?.pending_merges ?? []);
  let pendingReleases = $derived(plane?.pending_releases ?? []);
  let unreviewed = $derived(plane?.unreviewed ?? []);
  let mergeable = $derived(plane?.mergeable ?? []);
  let releasable = $derived(plane?.releasable ?? []);
  let stuck = $derived(plane?.stuck ?? []);

  let carrying = $derived(
    pendingReviews.length +
      pendingMerges.length +
      pendingReleases.length +
      unreviewed.length +
      mergeable.length +
      releasable.length +
      stuck.length,
  );

  // A readout holding nothing is not drawn, so a quiet pipeline says so in one
  // muted line rather than as an empty box.
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
  {:else if panelState === "empty"}
    <p class="state" data-block="idle">運んでいるものはありません</p>
  {:else}
    {#if pendingReviews.length > 0}
      <Readout
        name="reviews"
        block="reviews"
        caption="review 待ち"
        items={pendingReviews}
      />
    {/if}
    <MergeTrains pending={pendingMerges} />
    <Releases pending={pendingReleases} />
    <Reconciliation {unreviewed} {mergeable} {releasable} {stuck} />
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
</style>
