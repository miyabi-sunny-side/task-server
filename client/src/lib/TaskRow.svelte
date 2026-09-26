<script lang="ts">
  import type { TaskSummary } from "./api";
  import { blockedByLabel, executionTargetLabel } from "./api";

  import { tick } from "svelte";
  import { postTaskStatus } from "./api";
  import Modal from "./Modal.svelte";
  import RowMenu from "./RowMenu.svelte";

  let {
    item,
    onupdated,
  }: {
    item: TaskSummary;
    onupdated?: (task: TaskSummary) => void | Promise<void>;
  } = $props();
  let rowMenu = $state<RowMenu>();
  let row = $state<HTMLAnchorElement>();
  let menuOpen = $state(false);
  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  let confirming = $state(false);
  const errorId = $props.id();
  let canReady = $derived(
    item.status === "draft" && !item.archived && !!onupdated,
  );

  function canChange(status: string) {
    return !item.archived && !!onupdated && item.status !== status;
  }

  function askCancel() {
    if (busy || !canChange("cancelled")) return;
    rowMenu?.close();
    error = "";
    notice = "";
    confirming = true;
  }

  async function copyUrl() {
    if (busy || !row) return;
    busy = true;
    error = "";
    notice = "";
    try {
      await navigator.clipboard.writeText(row.href);
      notice = "URLをコピーしました";
    } catch {
      error = "URLのコピーに失敗しました";
    } finally {
      busy = false;
    }
  }

  async function changeStatus(status: string) {
    if (busy || !canChange(status) || (status === "ready" && !canReady)) return;
    busy = true;
    error = "";
    try {
      const updated = await postTaskStatus(item.id, status);
      const restoreFocus =
        menuOpen || !!confirming || document.activeElement === row;
      menuOpen = false;
      confirming = false;
      await onupdated?.(updated);
      await tick();
      if (
        restoreFocus &&
        (document.activeElement === document.body ||
          document.activeElement === row)
      )
        (
          document.getElementById(`task-${item.id}`) ??
          document.querySelector<HTMLElement>('header a[href="/closed"]')
        )?.focus({ preventScroll: true });
    } catch (cause) {
      error = cause instanceof Error ? cause.message : "操作に失敗しました";
    } finally {
      busy = false;
    }
  }
</script>

<RowMenu
  bind:this={rowMenu}
  bind:row
  bind:open={menuOpen}
  href={`/tasks/${encodeURIComponent(item.id)}`}
  id={`task-${item.id}`}
  label={item.title}
  closeLabel="タスクメニューを閉じる"
  {error}
  {errorId}
  errorInDialog={confirming}
  {notice}
>
  <span class="head">
    <span class="product product-first">{item.product_id}</span>
    <span class="product execution-target" data-field="execution-target"
      >{executionTargetLabel(item)}</span
    >
  </span>
  <span class="name">{item.title}</span>
  {#if item.status === "ready" && item.depends_on && item.dependency_status}
    <!-- Why a ready task is not being worked on, read off the list
         (DESIGN.md, Dependency): one muted line, gone once the dependency
         lands. Plain text — the card is the link. -->
    <span class="waiting" data-waiting-on={item.depends_on}
      >waiting depends_on: {item.depends_on}</span
    >
  {/if}
  <span class="tail">
    <span class="badge">{item.status}</span>
    {#if item.status === "blocked" && item.blocked_by}
      <!-- Who stopped it: a parked task (保留) is a decision, the other two
           are jams (DESIGN.md, Status is worn). -->
      <span class="badge" data-blocked-by={item.blocked_by}
        >{blockedByLabel(item.blocked_by)}</span
      >
    {/if}
    {#if item.kind !== "normal"}
      <span class="badge">{item.kind}</span>
    {/if}
  </span>
  {#snippet items()}
    {#if canReady}
      <button
        class="menu-item"
        role="menuitem"
        type="button"
        disabled={busy}
        onclick={() => changeStatus("ready")}>Readyにする</button
      >
    {/if}
    {#each ["blocked", "cancelled"] as status}
      {#if canChange(status)}
        <button
          class="menu-item"
          role="menuitem"
          type="button"
          disabled={busy}
          onclick={() =>
            status === "blocked" ? changeStatus(status) : askCancel()}
          >{status === "blocked" ? "Blockする" : "Cancelする"}</button
        >
      {/if}
    {/each}
    <button
      class="menu-item"
      role="menuitem"
      type="button"
      disabled={busy}
      onclick={copyUrl}>URLをコピー</button
    >
  {/snippet}
</RowMenu>

{#if confirming}
  <Modal
    title="Cancelする"
    onclose={() => {
      if (!busy) confirming = false;
    }}
  >
    <form
      onsubmit={(event) => {
        event.preventDefault();
        if (confirming) void changeStatus("cancelled");
      }}
    >
      <p class="confirmation">
        「{item.title}」をキャンセルしますか？
      </p>
      {#if error}<p id={errorId} class="error-banner" role="alert">
          {error}
        </p>{/if}
      <button
        class="btn"
        type="button"
        data-autofocus
        aria-disabled={busy}
        onclick={() => {
          if (!busy) confirming = false;
        }}>取りやめ</button
      >
      <button class="btn" type="submit" aria-disabled={busy}>Cancelする</button>
    </form>
  </Modal>
{/if}

<style lang="sass">
  .confirmation
    overflow-wrap: anywhere

  .error-banner
    overflow-wrap: anywhere

  .name
    overflow-wrap: anywhere

  .head
    display: flex
    flex-wrap: wrap
    align-items: baseline
    gap: var(--sp-2)

  // The product is the first thing read, so it is body-colored and small
  // rather than the muted caption the row recipe gives it.
  .product-first
    font-size: var(--fs-sm)
    line-height: 1.4
    color: var(--c-on-surface)

  // The wait is a fact about the task, said in the caption voice: muted,
  // small, one line under the title.
  .waiting
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .tail
    display: flex
    flex-wrap: wrap
    align-items: baseline
    gap: var(--sp-2)
</style>
