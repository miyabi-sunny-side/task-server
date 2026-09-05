<script lang="ts">
  import type { TaskSummary } from "./api";
  import TaskRow from "./TaskRow.svelte";

  const STATUS_ORDER = ["draft", "ready", "wip", "blocked"];

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

  let elsewhere = $derived(new Set(drawnElsewhere));
  let open = $derived(
    items.filter(
      (item) =>
        !item.archived && item.kind === "normal" && !elsewhere.has(item.id),
    ),
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
              <TaskRow {item} />
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

</style>
