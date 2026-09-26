<script lang="ts">
  import { createIdea, fetchIdeas, type IdeaSummary } from "../lib/api";
  import { tick } from "svelte";
  import { startAutoReload } from "../lib/auto-reload";
  import IdeaRow from "../lib/IdeaRow.svelte";

  let { archived }: { archived: boolean } = $props();

  let items = $state<IdeaSummary[]>([]);
  let fetchState = $state<"loading" | "error" | "ready">("loading");
  let controller: AbortController | undefined;
  let loaded = false;

  let draft = $state("");
  let adding = $state(false);
  let addError = $state("");
  let input = $state<HTMLInputElement | undefined>();
  // The page link to the other list, where a moved row went.
  let pageLink = $state<HTMLAnchorElement>();

  async function load() {
    controller?.abort();
    controller = new AbortController();
    if (!loaded) {
      fetchState = "loading";
    }
    try {
      items = await fetchIdeas(archived, controller.signal);
      fetchState = "ready";
      loaded = true;
    } catch (error) {
      // As on the other lists: a failed background reload keeps the drawn
      // rows; only a first load with nothing to keep says so.
      if (
        !loaded &&
        !(error instanceof DOMException && error.name === "AbortError")
      ) {
        fetchState = "error";
      }
    }
  }

  // A title is enough to keep a thought; the body grows later on its page.
  async function add(event: SubmitEvent) {
    event.preventDefault();
    const title = draft.trim();
    if (adding || !title) return;
    adding = true;
    addError = "";
    try {
      await createIdea({ title });
      draft = "";
      await load();
    } catch (error) {
      addError = error instanceof Error ? error.message : "追加に失敗しました";
    } finally {
      adding = false;
      // Disabling the field during the request dropped its focus; give it
      // back so the next idea can be typed at once.
      await tick();
      input?.focus();
    }
  }

  // Drop the row at once, then reload so the list matches the server.
  async function removeMoved(id: string) {
    items = items.filter((item) => item.id !== id);
    await load();
  }

  let listState = $derived(
    fetchState === "ready"
      ? items.length === 0
        ? "empty"
        : "success"
      : fetchState,
  );

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
  {#if archived}
    <div class="controls archive-head">
      <h1 class="heading">アーカイブ</h1>
      <a class="quiet-link" href="/ideas" bind:this={pageLink}>アイデア一覧</a>
    </div>
  {:else}
    <form class="controls add" onsubmit={add}>
      <label class="add-field">
        <span class="label">新しいアイデア</span>
        <input
          bind:value={draft}
          bind:this={input}
          disabled={adding}
          maxlength="200"
          autocomplete="off"
        />
      </label>
      <button
        class="btn"
        class:primary={!!draft.trim() && !adding}
        type="submit"
        disabled={adding}
        aria-disabled={!draft.trim()}>追加</button
      >
      <a class="quiet-link" href="/ideas/archived" bind:this={pageLink}
        >アーカイブ</a
      >
      {#if addError}
        <p class="error-banner add-error" role="alert">{addError}</p>
      {/if}
    </form>
  {/if}

  <section class="list" data-region="ideas" data-state={listState}>
    {#if listState === "loading"}
      <p class="state">
        <span class="spinner" aria-hidden="true"></span>読み込み中…
      </p>
    {:else if listState === "empty"}
      <p class="state">
        {archived
          ? "アーカイブしたアイデアはありません"
          : "アイデアがありません"}
      </p>
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
            <IdeaRow
              {item}
              landing={pageLink}
              onmoved={() => removeMoved(item.id)}
            />
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

<style lang="sass">
  // Screen-level controls: the first block of the column, never a band.
  .controls
    display: flex
    flex-wrap: wrap
    align-items: flex-end
    gap: var(--sp-2)
    margin-bottom: var(--sp-4)

  .archive-head
    align-items: center

  .add-field
    display: flex
    flex: 1 1 12rem
    flex-direction: column
    gap: var(--sp-1)
    min-width: 0

  .label
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  input
    width: 100%
    min-width: 0
    padding: var(--sp-2)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface)
    color: var(--c-on-surface)
    font-size: var(--fs-lg)
    line-height: 1.2

  input:focus
    border-color: var(--c-accent)

  .add .btn
    min-height: 40px

  .add-error
    flex-basis: 100%

  .heading
    flex: 1
    margin: 0
    font-size: var(--fs-md)
    font-weight: 500
    line-height: 1.2

  // A page link, not an action: link ink, the header link's hit target.
  .quiet-link
    display: inline-flex
    align-items: center
    min-height: 40px
    padding: 0 var(--sp-1)
    font-size: var(--fs-sm)

</style>
