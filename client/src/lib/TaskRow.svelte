<script lang="ts">
  import type { TaskSummary } from "./api";
  import { blockedByLabel } from "./api";

  // The one task card every list on the top page draws (DESIGN.md, Task
  // list): the product first, so the reader knows whose task this is before
  // the title; the title next, wrapping rather than clipping; the status
  // badge last, with who blocked it and the kind beside it. The whole card is
  // the link and the only focus stop — plain spans inside, never a nested
  // anchor. A status group and a readout draw the same card, so a task reads
  // the same wherever it sits.
  let { item }: { item: TaskSummary } = $props();
</script>

<a class="card stack" href={`/tasks/${item.id}`}>
  <span class="head">
    <span class="product product-first">{item.product_id}</span>
  </span>
  <span class="name">{item.title}</span>
  {#if item.status === "ready" && item.depends_on && item.dependency_status}
    <!-- Why a ready task is not being worked on, read off the list
         (DESIGN.md, Dependency): one muted line, gone once the dependency
         lands. Plain text — the card is the link. -->
    <span class="waiting" data-waiting-on={item.depends_on}
      >waiting depends_on: {item.depends_on}</span
    >
  {/if}
  <span class="tail">
    <span class="badge">{item.status}</span>
    {#if item.status === "blocked" && item.blocked_by}
      <!-- Who stopped it: a parked task (保留) is a decision, the other two
           are jams (DESIGN.md, Status is worn). -->
      <span class="badge" data-blocked-by={item.blocked_by}
        >{blockedByLabel(item.blocked_by)}</span
      >
    {/if}
    {#if item.kind !== "normal"}
      <span class="badge">{item.kind}</span>
    {/if}
  </span>
</a>

<style lang="sass">
  // The family card recipe lays its children out in a row; this card reads
  // top to bottom instead, so it stacks and lets the title wrap.
  .stack
    flex-direction: column
    align-items: stretch
    gap: var(--sp-1)

  .stack .name
    overflow-wrap: anywhere

  .head
    display: flex
    flex-wrap: wrap
    align-items: baseline
    gap: var(--sp-2)

  // The product is the first thing read, so it is body-colored and small
  // rather than the muted caption the row recipe gives it.
  .product-first
    font-size: var(--fs-sm)
    line-height: 1.4
    color: var(--c-on-surface)

  // The wait is a fact about the task, said in the caption voice: muted,
  // small, one line under the title.
  .waiting
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .tail
    display: flex
    flex-wrap: wrap
    align-items: baseline
    gap: var(--sp-2)
</style>
