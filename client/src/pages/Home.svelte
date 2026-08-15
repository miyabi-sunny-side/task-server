<script lang="ts">
  import { fetchTasks, type TaskSummary } from "../lib/api";

  type ListState = "loading" | "empty" | "error" | "success";

  let items = $state<TaskSummary[]>([]);
  let listState = $state<ListState>("loading");

  let controller: AbortController | undefined;
  let loadedOnce = false;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    if (!loadedOnce) {
      listState = "loading";
    }
    try {
      items = await fetchTasks(controller.signal);
      listState = items.length === 0 ? "empty" : "success";
      loadedOnce = true;
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      listState = "error";
    }
  }

  function onVisibilityChange() {
    if (document.visibilityState === "visible") {
      void load();
    }
  }

  $effect(() => {
    void load();
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  });
</script>

<section class="content" data-state={listState}>
  {#if listState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if listState === "empty"}
    <p class="state">タスクがありません</p>
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
        <li>
          <a class="card" href={`/tasks/${item.id}`}>
            <span class="name">{item.title}</span>
            <span class="tags">
              {#if item.kind === "instant:merge"}
                <span class="badge">instant:merge</span>
              {/if}
              <span class="updated">{item.status}</span>
            </span>
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style lang="sass">
  .cards
    display: flex
    flex-direction: column
    gap: var(--sp-2)
    margin: 0
    padding: 0
    list-style: none

  .card
    display: flex
    align-items: baseline
    justify-content: space-between
    gap: var(--sp-2)
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)
    color: var(--c-on-surface)
    text-decoration: none

    &:hover
      background: var(--c-hover-1)

  .name
    font-size: var(--fs-md)
    font-weight: 500

  .tags
    display: flex
    flex-shrink: 0
    align-items: baseline
    gap: var(--sp-2)

  .badge
    padding: var(--sp-1) var(--sp-2)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-full)
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .updated
    flex-shrink: 0
    font-size: var(--fs-xs)
    color: var(--c-muted)
</style>
