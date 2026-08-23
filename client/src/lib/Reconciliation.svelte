<script lang="ts">
  import type { TaskSummary } from "./api";
  import Readout from "./Readout.svelte";

  // The work the automation should be carrying and is not. Both sets are
  // empty in a healthy pipeline, which is what earns this the panel's one
  // danger frame — and `role="status"` rather than `role="alert"`, because it
  // is a state that has moved in, not the outcome of a request the operator
  // just made (DESIGN.md, Reconciliation).
  let {
    unreviewed = [],
    mergeable = [],
  }: {
    unreviewed?: TaskSummary[];
    mergeable?: TaskSummary[];
  } = $props();

  let stranded = $derived(unreviewed.length + mergeable.length);
</script>

{#if stranded > 0}
  <section
    class="stranded error-banner"
    data-block="reconciliation"
    role="status"
  >
    {#if unreviewed.length > 0}
      <Readout
        name="unreviewed"
        caption="review が発行されていない task"
        items={unreviewed}
        tone="danger"
      />
    {/if}
    {#if mergeable.length > 0}
      <Readout
        name="mergeable"
        caption="merge が発行されていない task"
        items={mergeable}
        tone="danger"
      />
    {/if}
  </section>
{/if}

<style lang="sass">
  .stranded
    display: flex
    flex-direction: column
    gap: var(--sp-3)
</style>
