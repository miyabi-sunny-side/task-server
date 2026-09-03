<script lang="ts">
  import type { TaskSummary } from "./api";

  // DESIGN.md, Task list: the status vocabulary order, which is the pipeline
  // read from its start — so `approved` sits between `done` and `merged`,
  // past review and not yet landed. `released` is absent on purpose —
  // shipped work leaves the top page.
  const STATUS_ORDER = [
    "draft",
    "ready",
    "wip",
    "done",
    "approved",
    "merged",
    "blocked",
    "cancelled",
    "dropped",
  ];

  let {
    fetchState,
    items = [],
    drawnElsewhere = [],
    onretry,
  }: {
    fetchState: "loading" | "error" | "ready";
    items?: TaskSummary[];
    // The ids the control panel is already drawing in its readouts: the
    // pending reviews and everything stranded in reconciliation.
    drawnElsewhere?: string[];
    onretry?: () => void;
  } = $props();

  // A subtask (kind other than `normal`) exists on this page only while the
  // panel is drawing it in a readout — the review queue, the merge trains, or
  // reconciliation. It never falls into a status group, finished or not: a
  // `review` verdict lives on its target's `latest_review`, so a husk card
  // once the panel stops drawing it would carry no information the detail
  // page does not already have. A `normal` task hides only while another
  // region of this same page already draws it, because one object drawn
  // twice on one screen reads as two; it falls back into its status group
  // the moment the panel stops.
  let elsewhere = $derived(new Set(drawnElsewhere));
  let open = $derived(
    items.filter((item) => item.kind === "normal" && !elsewhere.has(item.id)),
  );

  let groups = $derived(
    STATUS_ORDER.map((status) => ({
      status,
      items: open.filter((item) => item.status === status),
    })).filter((group) => group.items.length > 0),
  );

  let listState = $derived(
    fetchState === "ready"
      ? groups.length === 0
        ? "empty"
        : "success"
      : fetchState,
  );
</script>

<section class="list" data-region="tasks" data-state={listState}>
  {#if listState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if listState === "empty"}
    <p class="state">タスクがありません</p>
  {:else if listState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => onretry?.()}>
        再試行
      </button>
    </div>
  {:else}
    {#each groups as group (group.status)}
      <section class="group" data-status={group.status}>
        <h2 class="group-head">
          <span class="group-name">{group.status}</span>
          <span class="pill" data-count>{group.items.length}</span>
        </h2>
        <ul class="cards">
          {#each group.items as item (item.id)}
            <li>
              <a class="card" href={`/tasks/${item.id}`}>
                <span class="name">{item.title}</span>
                <span class="tail">
                  {#if item.depends_on}
                    <!-- Why a draft is still a draft, read off the list. -->
                    <span class="product" data-depends-on={item.depends_on}
                      >← {item.depends_on}</span
                    >
                  {/if}
                  <span class="product">{item.product_id}</span>
                </span>
              </a>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  {/if}
</section>

<style lang="sass">
  .list
    display: flex
    flex-direction: column
    gap: var(--sp-5)

  .group-head
    display: flex
    align-items: center
    gap: var(--sp-2)
    margin: 0 0 var(--sp-2)

  .group-name
    font-size: var(--fs-md)
    font-weight: 500
    line-height: 1.2

  .tail
    display: flex
    flex-shrink: 0
    align-items: baseline
    gap: var(--sp-2)
</style>
