<script lang="ts">
  import type { TaskSummary } from "./api";
  import TaskRow from "./TaskRow.svelte";

  // One readout: a muted caption with its count pill over the ordinary card
  // list, exactly like a status-group heading (DESIGN.md, Readouts). The
  // caller renders it only while it holds something — an empty readout is not
  // drawn at all, caption included. The cards are the status-group card, so a
  // task reads the same wherever it sits.
  let {
    name,
    caption,
    items,
    tone = "muted",
    block,
  }: {
    name: string;
    caption: string;
    items: TaskSummary[];
    // `info` is for the reconciliation frame alone, whose text is the info
    // token on info-subtle; every other readout is neutral.
    tone?: "muted" | "info";
    block?: string;
  } = $props();
</script>

<div class="readout" data-readout={name} data-block={block}>
  <p class="head">
    <span class="caption" class:info={tone === "info"}>{caption}</span>
    <span class="pill" data-count>{items.length}</span>
  </p>
  <ul class="cards">
    {#each items as item (item.id)}
      <li>
        <TaskRow {item} />
      </li>
    {/each}
  </ul>
</div>

<style lang="sass">
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

  .caption.info
    color: var(--c-info)
</style>
