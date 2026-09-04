<script lang="ts">
  import type { Releasable, Stuck, StuckReason, TaskSummary } from "./api";
  import Readout from "./Readout.svelte";
  import TaskRow from "./TaskRow.svelte";

  // The work the automation should be carrying and is not. Every set is
  // empty in a healthy pipeline, which is what earns this the panel's one
  // info frame — a standing state a person should look at, not a failure, so
  // never the danger pair — and `role="status"` rather than `role="alert"`,
  // because it has moved in rather than answered a request the operator just
  // made (DESIGN.md, Reconciliation).
  let {
    unreviewed = [],
    mergeable = [],
    releasable = [],
    stuck = [],
    tasks = [],
  }: {
    unreviewed?: TaskSummary[];
    mergeable?: TaskSummary[];
    releasable?: Releasable[];
    stuck?: Stuck[];
    // The page's task list, so a stuck row (which names only its task) can be
    // drawn as the ordinary card with its product and title.
    tasks?: TaskSummary[];
  } = $props();

  let stranded = $derived(
    unreviewed.length + mergeable.length + releasable.length + stuck.length,
  );

  // What each reason means, in the words DESIGN.md fixes (Stuck): a caption
  // and one line on what to do about it.
  const WORDING: Record<StuckReason, { caption: string; note: string }> = {
    blocked: {
      caption: "長時間 blocked 状態",
      note: "追加の議論が必要でしょうか。ご確認ください",
    },
    unclaimed: {
      caption: "長時間 claim されていない",
      note: "worker が動いているかご確認ください",
    },
    "lease-expired": {
      caption: "lease が切れた",
      note: "worker が途中で止まった可能性があります。ご確認ください",
    },
    "no-subtask": {
      caption: "次の subtask が発行されていない",
      note: "review / merge / release の発行が止まっています。ご確認ください",
    },
    "subtask-unclaimed": {
      caption: "subtask が claim されていない",
      note: "subtask を拾う worker が動いているかご確認ください",
    },
    "release-stalled": {
      caption: "release が進んでいない",
      note: "release task が止まっています。ご確認ください",
    },
  };

  let byId = $derived(new Map(tasks.map((task) => [task.id, task])));

  // A stuck row as the card the status groups draw. The list the page holds
  // supplies product and title; a task it does not carry is drawn from what
  // the server said, with no invented product.
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

  // One group per reason, in the order the server first named each; the
  // server sorted the rows, the screen sorts nothing.
  let groups = $derived.by(() => {
    const order: StuckReason[] = [];
    const rows = new Map<StuckReason, Stuck[]>();
    for (const row of stuck) {
      if (!rows.has(row.reason)) {
        order.push(row.reason);
        rows.set(row.reason, []);
      }
      rows.get(row.reason)!.push(row);
    }
    return order.map((reason) => ({ reason, rows: rows.get(reason)! }));
  });
</script>

{#if stranded > 0}
  <section
    class="stranded info-banner"
    data-block="reconciliation"
    role="status"
  >
    {#if unreviewed.length > 0}
      <Readout
        name="unreviewed"
        caption="review が発行されていない task"
        items={unreviewed}
        tone="info"
      />
    {/if}
    {#if mergeable.length > 0}
      <Readout
        name="mergeable"
        caption="merge が発行されていない task"
        items={mergeable}
        tone="info"
      />
    {/if}
    {#if releasable.length > 0}
      <!-- Landed work with no release carrying it, per product. A product is
           the unit here, not a task: one release ships all of it. -->
      <div class="readout" data-readout="releasable">
        <p class="head">
          <span class="caption info">release が発行されていない product</span>
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
    {#if stuck.length > 0}
      <!-- Work the server measured as waiting past its threshold. The server
           decided (clock and threshold); the screen only states it: one group
           per reason, said in plain words with a line on what to do, and under
           it the ordinary task cards. No button: the rescue is a person's or a
           script's, never this page's. -->
      <div class="readout" data-readout="stuck">
        {#each groups as group (group.reason)}
          <div class="group" data-reason={group.reason}>
            <p class="head">
              <span class="caption info">{WORDING[group.reason].caption}</span>
              <span class="pill" data-count>{group.rows.length}</span>
            </p>
            <p class="note">{WORDING[group.reason].note}</p>
            <ul class="cards">
              {#each group.rows as row (row.task_id)}
                <li>
                  <TaskRow item={asCard(row)} />
                </li>
              {/each}
            </ul>
          </div>
        {/each}
      </div>
    {/if}
  </section>
{/if}

<style lang="sass">
  .stranded
    display: flex
    flex-direction: column
    gap: var(--sp-3)

  .readout, .group
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

  // The line under a reason's caption: what to do about it, in the caption
  // voice, one line.
  .note
    margin: 0
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .name
    overflow-wrap: anywhere

  .tail
    display: flex
    flex-shrink: 0
    align-items: baseline
    gap: var(--sp-2)
</style>
