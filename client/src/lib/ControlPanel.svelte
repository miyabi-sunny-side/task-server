<script lang="ts">
  import type { ControlPlane, TaskSummary } from "./api";
  import Reconciliation from "./Reconciliation.svelte";

  let {
    fetchState,
    plane,
    tasks = [],
    onretry,
  }: {
    fetchState: "loading" | "error" | "ready";
    plane?: ControlPlane;
    tasks?: TaskSummary[];
    onretry?: () => void;
  } = $props();
  let stuck = $derived(
    (plane?.stuck ?? []).filter(
      (row) => row.reason === "blocked" || row.reason === "lease-expired",
    ),
  );
  let panelState = $derived(
    fetchState === "ready" ? (stuck.length ? "success" : "empty") : fetchState,
  );
</script>

<section data-region="control" data-state={panelState}>
  {#if panelState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if panelState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => onretry?.()}
        >再試行</button
      >
    </div>
  {:else if panelState === "success"}
    <Reconciliation {stuck} {tasks} />
  {/if}
</section>

<style lang="sass">
  section:not([data-state="empty"])
    margin-bottom: var(--sp-5)
</style>
