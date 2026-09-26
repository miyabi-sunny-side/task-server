<script lang="ts">
  import { tick } from "svelte";
  import { archiveIdea, type Idea, type IdeaSummary } from "./api";
  import IdeaArchiveDialog from "./IdeaArchiveDialog.svelte";
  import RowMenu from "./RowMenu.svelte";

  // Without `onarchived` (the archive page) the row is a plain link.
  let {
    item,
    onarchived,
  }: {
    item: IdeaSummary;
    onarchived?: (idea: Idea) => void | Promise<void>;
  } = $props();
  let rowMenu = $state<RowMenu>();
  let row = $state<HTMLAnchorElement>();
  let menuOpen = $state(false);
  let busy = $state(false);
  let error = $state("");
  let confirming = $state(false);
  const errorId = $props.id();
  let href = $derived(`/ideas/${encodeURIComponent(item.id)}`);

  function askArchive() {
    if (busy) return;
    rowMenu?.close();
    error = "";
    confirming = true;
  }

  async function archive() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      const idea = await archiveIdea(item.id);
      const restoreFocus = confirming || document.activeElement === row;
      menuOpen = false;
      confirming = false;
      await onarchived?.(idea);
      await tick();
      // The idea left this list for the archive; focus goes where it went,
      // as a task leaving the page focuses the closed link.
      if (
        restoreFocus &&
        (document.activeElement === document.body ||
          document.activeElement === row)
      )
        document
          .querySelector<HTMLElement>('a[href="/ideas/archived"]')
          ?.focus({ preventScroll: true });
    } catch (cause) {
      error =
        cause instanceof Error ? cause.message : "アーカイブに失敗しました";
    } finally {
      busy = false;
    }
  }
</script>

{#snippet content()}
  {#if item.product_id}
    <span class="product idea-product">{item.product_id}</span>
  {/if}
  <span class="name">{item.title}</span>
  <span class="tail">
    <span class="at"
      >{item.archived
        ? (item.archived_at ?? item.updated_at)
        : item.updated_at}</span
    >
    {#if item.task_id}<span class="badge">タスク化済み</span>{/if}
  </span>
{/snippet}

{#if onarchived}
  <RowMenu
    bind:this={rowMenu}
    bind:row
    bind:open={menuOpen}
    {href}
    id={`idea-${item.id}`}
    label={item.title}
    closeLabel="アイデアメニューを閉じる"
    {error}
    {errorId}
    errorInDialog={confirming}
  >
    {@render content()}
    {#snippet items()}
      <button
        class="menu-item"
        role="menuitem"
        type="button"
        disabled={busy}
        onclick={askArchive}>アーカイブ</button
      >
    {/snippet}
  </RowMenu>
{:else}
  <a class="card plain" {href}>{@render content()}</a>
{/if}

{#if confirming}
  <IdeaArchiveDialog
    title={item.title}
    {busy}
    {error}
    {errorId}
    onconfirm={() => void archive()}
    onclose={() => (confirming = false)}
  />
{/if}

<style lang="sass">
  .plain
    flex-direction: column
    align-items: stretch
    gap: var(--sp-1)

  .idea-product
    color: var(--c-muted)

  .name
    overflow-wrap: anywhere

  .tail
    display: flex
    flex-wrap: wrap
    align-items: baseline
    gap: var(--sp-1) var(--sp-2)

  .at
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)
</style>
