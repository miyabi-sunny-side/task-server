<script lang="ts">
  import type { Releasable, TaskSummary } from "./api";
  import Readout from "./Readout.svelte";

  // The work the automation should be carrying and is not. Every set is
  // empty in a healthy pipeline, which is what earns this the panel's one
  // danger frame — and `role="status"` rather than `role="alert"`, because it
  // is a state that has moved in, not the outcome of a request the operator
  // just made (DESIGN.md, Reconciliation).
  let {
    unreviewed = [],
    mergeable = [],
    releasable = [],
  }: {
    unreviewed?: TaskSummary[];
    mergeable?: TaskSummary[];
    releasable?: Releasable[];
  } = $props();

  let stranded = $derived(
    unreviewed.length + mergeable.length + releasable.length,
  );
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
    {#if releasable.length > 0}
      <!-- Landed work with no release carrying it, per product. A product is
           the unit here, not a task: one release ships all of it. -->
      <div class="readout" data-readout="releasable">
        <p class="head">
          <span class="caption danger">release が発行されていない product</span>
          <span class="pill" data-count>{releasable.length}</span>
        </p>
        <ul class="cards">
          {#each releasable as item (item.product_id)}
            <li>
              <span class="card" data-product={item.product_id}>
                <span class="name">{item.product_id}</span>
                <span class="tail">
                  <span class="pill" data-count>{item.task_count}</span>
                </span>
              </span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </section>
{/if}

<style lang="sass">
  .stranded
    display: flex
    flex-direction: column
    gap: var(--sp-3)

  .readout
    display: flex
    flex-direction: column
    gap: var(--sp-2)

  .head
    display: flex
    align-items: center
    gap: var(--sp-2)
    margin: 0

  .caption
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)
    overflow-wrap: anywhere

  .caption.danger
    color: var(--c-danger)

  .name
    overflow-wrap: anywhere

  .tail
    display: flex
    flex-shrink: 0
    align-items: baseline
    gap: var(--sp-2)
</style>
