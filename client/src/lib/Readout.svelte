<script lang="ts">
  import type { TaskSummary } from "./api";

  // One readout: a muted caption with its count pill over the ordinary card
  // list, exactly like a status-group heading (DESIGN.md, Readouts). The
  // caller renders it only while it holds something — an empty readout is not
  // drawn at all, caption included.
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
    // `danger` is for the reconciliation frame alone, whose text is the
    // danger token on danger-subtle; every other readout is neutral.
    tone?: "muted" | "danger";
    block?: string;
  } = $props();
</script>

<div class="readout" data-readout={name} data-block={block}>
  <p class="head">
    <span class="caption" class:danger={tone === "danger"}>{caption}</span>
    <span class="pill" data-count>{items.length}</span>
  </p>
  <ul class="cards">
    {#each items as item (item.id)}
      <li>
        <a class="card" href={`/tasks/${item.id}`}>
          <span class="name">{item.title}</span>
          <span class="tail">
            <!-- No status heading sits over a readout, so its cards wear the
                 neutral outline status badge. Plain spans: the badge never
                 adds a second focus stop inside the card link. -->
            <span class="badge">{item.status}</span>
            <span class="product">{item.product_id}</span>
          </span>
        </a>
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

  .caption.danger
    color: var(--c-danger)

  .tail
    display: flex
    flex-shrink: 0
    align-items: baseline
    gap: var(--sp-2)
</style>
