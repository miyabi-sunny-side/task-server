<script lang="ts">
  import { fetchClosed, type ClosedTask } from "../lib/api";
  import { startAutoReload } from "../lib/auto-reload";

  type FetchState = "loading" | "error" | "ready";

  let items = $state<ClosedTask[]>([]);
  let fetchState = $state<FetchState>("loading");
  let controller: AbortController | undefined;
  let loaded = false;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    if (!loaded) {
      fetchState = "loading";
    }
    try {
      items = await fetchClosed(controller.signal);
      fetchState = "ready";
      loaded = true;
    } catch (error) {
      // A background reload that fails leaves the drawn rows exactly as they
      // are (DESIGN.md, Do's and Don'ts); only the first load has nothing
      // to keep and says so.
      if (
        !loaded &&
        !(error instanceof DOMException && error.name === "AbortError")
      ) {
        fetchState = "error";
      }
    }
  }

  let listState = $derived(
    fetchState === "ready"
      ? items.length === 0
        ? "empty"
        : "success"
      : fetchState,
  );

  // DESIGN.md, Closed page: the list is the forest. The summary a person
  // wrote; failing that, the first line of the log cut at 80 code points; and
  // no element at all when there is nothing to show.
  function digest(item: ClosedTask): string {
    const summary = item.summary?.trim();
    if (summary) return summary;
    const first = (item.verification ?? "").split("\n")[0].trim();
    return Array.from(first).slice(0, 80).join("");
  }

  // The same rhythm as the top page and the detail: reload while visible,
  // reload at once on coming back to the tab, stop when the page unmounts.
  $effect(() => {
    void load();
    const stopAutoReload = startAutoReload(() => void load());
    return () => {
      stopAutoReload();
      controller?.abort();
    };
  });
</script>

<div class="content">
  <section class="list" data-region="closed" data-state={listState}>
    {#if listState === "loading"}
      <p class="state">
        <span class="spinner" aria-hidden="true"></span>読み込み中…
      </p>
    {:else if listState === "empty"}
      <p class="state">閉じたタスクがありません</p>
    {:else if listState === "error"}
      <div class="state-wrap">
        <p class="state error">読み込みに失敗しました</p>
        <button class="btn" type="button" onclick={() => void load()}>
          再試行
        </button>
      </div>
    {:else}
      <ul class="cards">
        {#each items as item (item.id)}
          {@const shown = digest(item)}
          <li>
            <!-- Forest before trees (DESIGN.md, Closed page): product, title,
                 the summary, then when and how it closed. The log never sits
                 on a list; the Task Card folds it away. -->
            <a class="card" href={`/tasks/${item.id}`}>
              <span class="product product-first">{item.product_id}</span>
              <span class="name">{item.title}</span>
              {#if shown}
                <span class="summary">{shown}</span>
              {/if}
              <span class="tail">
                <span class="done-at">{item.closed_at}</span>
                <span class="badge">{item.status}</span>
                {#if item.release_tag}
                  <span class="badge">{item.release_tag}</span>
                {/if}
              </span>
            </a>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

<style lang="sass">
  // The card recipe, stacked so a verification excerpt can sit under the
  // title (DESIGN.md, Closed page — the same shape as a blocked merge card).
  .card
    flex-direction: column
    align-items: stretch
    gap: var(--sp-2)

  // The product is the first thing read, so it is body-colored and small
  // rather than the muted caption the row recipe gives it.
  .product-first
    font-size: var(--fs-sm)
    line-height: 1.4
    color: var(--c-on-surface)

  .name
    overflow-wrap: anywhere

  .summary
    font-size: var(--fs-sm)
    line-height: 1.5
    color: var(--c-on-surface)
    overflow-wrap: anywhere

  .tail
    display: flex
    flex-wrap: wrap
    align-items: baseline
    gap: var(--sp-1) var(--sp-2)

  .done-at
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

</style>
