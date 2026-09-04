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

  // DESIGN.md, Closed page: the first one or two source lines, not a
  // CSS-clamped truncation, and no element at all when there is nothing to
  // show.
  function excerpt(verification: string | null): string {
    if (!verification || verification.trim() === "") {
      return "";
    }
    return verification.split("\n").slice(0, 2).join("\n");
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
          {@const shown = excerpt(item.verification)}
          <li>
            <a class="card" href={`/tasks/${item.id}`}>
              <span class="line">
                <span class="name">{item.title}</span>
                <span class="tail">
                  <span class="badge">{item.status}</span>
                  {#if item.release_tag}
                    <span class="badge">{item.release_tag}</span>
                  {/if}
                  <span class="product">{item.product_id}</span>
                  <span class="done-at">{item.closed_at}</span>
                </span>
              </span>
              {#if shown}
                <span class="excerpt">{shown}</span>
              {/if}
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

  .line
    display: flex
    flex-wrap: wrap
    align-items: baseline
    justify-content: space-between
    gap: var(--sp-1) var(--sp-2)

  .tail
    display: flex
    flex-wrap: wrap
    align-items: baseline
    gap: var(--sp-1) var(--sp-2)

  .done-at
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .excerpt
    font-size: var(--fs-sm)
    line-height: 1.5
    color: var(--c-muted)
    white-space: pre-line
    overflow-wrap: anywhere
</style>
