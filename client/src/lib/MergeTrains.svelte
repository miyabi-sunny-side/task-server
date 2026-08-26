<script lang="ts">
  import type { PendingMerge } from "./api";
  import { mergeTrains } from "./trains";

  // The outstanding merges, grouped by product: the train is per product, and
  // a single flat list would say one product's jam holds another's
  // (DESIGN.md, Merge trains). Each merge carries its own stop reason in
  // `verification`, written by the worker that could not integrate it, so the
  // jam is read off the same payload that drew the queue.
  let {
    pending = [],
  }: {
    pending?: PendingMerge[];
  } = $props();

  let trains = $derived(mergeTrains(pending));

  // A jam is legible only when its cause and its cost are both named, and
  // neither is worth saying about a train that is running. The cause belongs
  // to the stopped merge itself rather than to a position, so a reordering
  // can never move it onto the wrong card (DESIGN.md, Merge trains).
  function waitingOf(items: PendingMerge[]) {
    return items.some((item) => item.status === "blocked")
      ? items.filter((item) => item.status === "ready").length
      : 0;
  }
</script>

{#if trains.length > 0}
  <div class="trains" data-block="trains">
    {#each trains as train (train.productId)}
      {@const waiting = waitingOf(train.items)}
      <div class="train" data-train={train.productId}>
        <p class="head">
          <span class="caption">{train.productId}</span>
          <span class="pill" data-count>{train.items.length}</span>
        </p>
        <ul class="cards">
          {#each train.items as item (item.id)}
            <li>
              <a class="card" href={`/tasks/${item.id}`}>
                <span class="line">
                  <span class="name">{item.title}</span>
                  <span class="tail">
                    <!-- Every card wears its status badge like any other
                         readout card, so no caption repeats a status. -->
                    <span class="badge">{item.status}</span>
                  </span>
                </span>
                {#if item.status === "blocked" && (item.verification ?? "") !== ""}
                  <span class="reason" data-reason>{item.verification}</span>
                {/if}
              </a>
            </li>
          {/each}
        </ul>
        {#if waiting > 0}
          <p class="caption">他 {waiting} 件が待機中</p>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style lang="sass">
  .trains
    display: flex
    flex-direction: column
    gap: var(--sp-3)

  .train
    display: flex
    flex-direction: column
    gap: var(--sp-2)

  .head
    display: flex
    align-items: center
    gap: var(--sp-2)
    margin: 0

  .caption
    margin: 0
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)
    // Product ids are slash-joined and unbreakable by default; a narrow
    // viewport must never be pushed sideways by one.
    overflow-wrap: anywhere

  // The card recipe, stacked so a reason can sit under the title.
  .card
    flex-direction: column
    align-items: stretch
    gap: var(--sp-2)

  .line
    display: flex
    align-items: baseline
    justify-content: space-between
    gap: var(--sp-2)

  .tail
    display: flex
    flex-shrink: 0
    align-items: baseline
    gap: var(--sp-2)

  // Neutral throughout: a rebase conflict is an ordinary outcome of landing
  // work, not a failure of this app, so the danger tokens stay out.
  .reason
    font-size: var(--fs-sm)
    line-height: 1.5
    white-space: pre-line
    // Reasons quote shas, paths, and command lines.
    overflow-wrap: anywhere
</style>
