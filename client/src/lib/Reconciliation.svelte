<script lang="ts">
  import type { Stuck, TaskSummary } from "./api";
  import TaskRow from "./TaskRow.svelte";

  let { stuck = [], tasks = [] }: { stuck?: Stuck[]; tasks?: TaskSummary[] } =
    $props();
  const captions: Record<string, string> = {
    blocked: "実行が止まっています",
    "lease-expired": "実行の期限が切れています",
  };
  let byId = $derived(new Map(tasks.map((task) => [task.id, task])));
  let groups = $derived(
    [...new Set(stuck.map((row) => row.reason))]
      .filter((reason) => captions[reason])
      .map((reason) => ({
        reason,
        rows: stuck.filter((row) => row.reason === reason),
      })),
  );

  function asCard(row: Stuck): TaskSummary {
    return (
      byId.get(row.task_id) ?? {
        id: row.task_id,
        title: row.task_id,
        status: row.status,
        kind: row.kind,
        product_id: "",
        priority: 0,
        updated_at: row.since,
      }
    );
  }
</script>

{#if groups.length}
  <section
    class="stranded info-banner"
    data-block="reconciliation"
    role="status"
  >
    {#each groups as group (group.reason)}
      <div class="group" data-reason={group.reason}>
        <p class="head">
          <span>{captions[group.reason]}</span><span class="pill" data-count
            >{group.rows.length}</span
          >
        </p>
        <ul class="cards">
          {#each group.rows as row (row.task_id)}
            <li><TaskRow item={asCard(row)} /></li>
          {/each}
        </ul>
      </div>
    {/each}
  </section>
{/if}

<style lang="sass">
  .stranded, .group
    display: flex
    flex-direction: column
    gap: var(--sp-2)

  .head
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-2)
    margin: 0
    font-size: var(--fs-xs)
    line-height: 1.4
    overflow-wrap: anywhere
</style>
