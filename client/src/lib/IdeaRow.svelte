<script lang="ts">
  import { tick } from "svelte";
  import {
    archiveIdea,
    unarchiveIdea,
    type Idea,
    type IdeaSummary,
  } from "./api";
  import RowMenu from "./RowMenu.svelte";

  // The row menu moves an idea between the open list and the archive at
  // once: both directions are reversible, so neither asks first.
  let {
    item,
    landing,
    onmoved,
  }: {
    item: IdeaSummary;
    // Where focus goes once the row leaves: the page link to the list the
    // idea moved to.
    landing?: HTMLElement;
    onmoved: (idea: Idea) => void | Promise<void>;
  } = $props();
  let row = $state<HTMLAnchorElement>();
  let menuOpen = $state(false);
  let busy = $state(false);
  let error = $state("");
  const errorId = $props.id();
  let href = $derived(`/ideas/${encodeURIComponent(item.id)}`);

  async function move() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      const idea = await (item.archived ? unarchiveIdea : archiveIdea)(item.id);
      const restoreFocus = menuOpen || document.activeElement === row;
      menuOpen = false;
      await onmoved(idea);
      await tick();
      // As a task leaving the page focuses the closed link.
      if (
        restoreFocus &&
        (document.activeElement === document.body ||
          document.activeElement === row)
      )
        landing?.focus({ preventScroll: true });
    } catch (cause) {
      error =
        cause instanceof Error
          ? cause.message
          : item.archived
            ? "アイデアに戻せませんでした"
            : "アーカイブに失敗しました";
    } finally {
      busy = false;
    }
  }
</script>

<RowMenu
  bind:row
  bind:open={menuOpen}
  {href}
  id={`idea-${item.id}`}
  label={item.title}
  closeLabel="アイデアメニューを閉じる"
  {error}
  {errorId}
>
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
  {#snippet items()}
    <button
      class="menu-item"
      role="menuitem"
      type="button"
      disabled={busy}
      onclick={() => void move()}
      >{item.archived ? "アイデアに戻す" : "アーカイブ"}</button
    >
  {/snippet}
</RowMenu>

<style lang="sass">
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
