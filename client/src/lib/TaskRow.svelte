<script lang="ts">
  import type { TaskSummary } from "./api";
  import { blockedByLabel } from "./api";

  import { onMount, tick } from "svelte";
  import { postTaskStatus } from "./api";
  import Modal from "./Modal.svelte";

  let {
    item,
    onupdated,
  }: {
    item: TaskSummary;
    onupdated?: (task: TaskSummary) => void | Promise<void>;
  } = $props();
  let row: HTMLAnchorElement;
  let menu = $state<HTMLElement>();
  let menuOpen = $state(false);
  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  let confirming = $state<"blocked" | "cancelled">();
  let confirmLabel = $derived(
    confirming === "blocked" ? "Blockする" : "Cancelする",
  );
  let x = $state(0);
  let y = $state(0);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let origin: { x: number; y: number } | undefined;
  let held = false;
  let anchorTop = 0;
  const errorId = $props.id();
  let canReady = $derived(
    item.status === "draft" && !item.archived && !!onupdated,
  );

  function canChange(status: string) {
    return !item.archived && !!onupdated && item.status !== status;
  }

  function ask(status: "blocked" | "cancelled") {
    if (busy || !canChange(status)) return;
    closeMenu();
    error = "";
    notice = "";
    confirming = status;
  }

  async function copyUrl() {
    if (busy) return;
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
      await tick();
      if (menuOpen) placeMenu();
    }
  }

  function cancelPress() {
    clearTimeout(timer);
    origin = undefined;
  }

  function closeMenu() {
    if (!menuOpen) return;
    menuOpen = false;
    row.focus({ preventScroll: true });
  }

  function placeMenu() {
    if (!menu) return;
    const rect = row.getBoundingClientRect();
    anchorTop = rect.top;
    const bounds = menu.getBoundingClientRect();
    const gutter = parseFloat(
      getComputedStyle(menu).getPropertyValue("--sp-3"),
    );
    x = Math.max(
      gutter,
      Math.min(rect.left, innerWidth - bounds.width - gutter),
    );
    y = Math.max(
      gutter,
      Math.min(rect.bottom, innerHeight - bounds.height - gutter),
    );
  }

  async function openMenu() {
    cancelPress();
    if (menuOpen) return;
    menuOpen = true;
    anchorTop = row.getBoundingClientRect().top;
    await tick();
    if (!menuOpen || !menu) return;
    placeMenu();
    menu
      .querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)')
      ?.focus({ preventScroll: true });
  }

  function pointerDown(event: PointerEvent) {
    cancelPress();
    if (!event.isPrimary || event.button !== 0 || event.pointerType === "mouse")
      return;
    origin = { x: event.clientX, y: event.clientY };
    timer = setTimeout(() => {
      held = true;
      void openMenu();
    }, 500);
  }

  function pointerMove(event: PointerEvent) {
    if (
      origin &&
      Math.hypot(event.clientX - origin.x, event.clientY - origin.y) > 8
    )
      cancelPress();
  }

  function rowKeydown(event: KeyboardEvent) {
    if (
      event.key === "ContextMenu" ||
      (event.shiftKey && event.key === "F10")
    ) {
      event.preventDefault();
      void openMenu();
    }
  }

  function menuKeydown(event: KeyboardEvent) {
    if (!menuOpen) return;
    if (event.key === "Escape" || event.key === "Tab") {
      if (event.key === "Escape") event.preventDefault();
      closeMenu();
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key) || !menu)
      return;
    event.preventDefault();
    const entries = [
      ...menu.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)'),
    ];
    const index = entries.indexOf(document.activeElement as HTMLElement);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? entries.length - 1
          : (index + (event.key === "ArrowDown" ? 1 : -1) + entries.length) %
            entries.length;
    entries[next]?.focus();
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
      confirming = undefined;
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
      await tick();
      if (menuOpen) placeMenu();
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    // Touch browsers can retarget the release click to the new overlay.
    const resetClick = () => {
      held = false;
    };
    const suppressClick = (event: MouseEvent) => {
      if (!held) return;
      if (event.type === "click") held = false;
      event.preventDefault();
      event.stopPropagation();
    };
    document.addEventListener("pointerdown", resetClick, true);
    document.addEventListener("mousedown", suppressClick, true);
    document.addEventListener("click", suppressClick, true);
    const onscroll = (event: Event) => {
      cancelPress();
      // A scroll queued before opening must not dismiss a newly opened menu.
      if (
        menuOpen &&
        !(event.target instanceof Node && menu?.contains(event.target)) &&
        row.getBoundingClientRect().top !== anchorTop
      )
        closeMenu();
    };
    document.addEventListener("scroll", onscroll, true);
    return () => {
      cancelPress();
      document.removeEventListener("pointerdown", resetClick, true);
      document.removeEventListener("mousedown", suppressClick, true);
      document.removeEventListener("click", suppressClick, true);
      document.removeEventListener("scroll", onscroll, true);
    };
  });
</script>

<svelte:window onkeydown={menuKeydown} onresize={closeMenu} />

<a
  class="card stack"
  href={`/tasks/${encodeURIComponent(item.id)}`}
  id={`task-${item.id}`}
  bind:this={row}
  aria-haspopup="menu"
  aria-expanded={menuOpen}
  aria-describedby={error ? errorId : undefined}
  onpointerdown={pointerDown}
  onpointermove={pointerMove}
  onpointerup={cancelPress}
  onpointerleave={cancelPress}
  onpointercancel={cancelPress}
  onkeydown={rowKeydown}
  oncontextmenu={(event) => {
    event.preventDefault();
    void openMenu();
  }}
>
  <span class="head">
    <span class="product product-first">{item.product_id}</span>
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
</a>

{#if notice && !menuOpen}<p role="status">{notice}</p>{/if}
{#if error && !menuOpen && !confirming}
  <p id={errorId} class="error-banner" role="alert">{error}</p>
{/if}
{#if menuOpen}
  <button
    class="menu-overlay"
    type="button"
    tabindex="-1"
    aria-label="タスクメニューを閉じる"
    onclick={closeMenu}
  ></button>
  <div
    class="menu task-menu"
    bind:this={menu}
    style:left={`${x}px`}
    style:top={`${y}px`}
  >
    <div role="menu" aria-label={item.title}>
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
            onclick={() => ask(status as "blocked" | "cancelled")}
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
    </div>
    {#if notice}<p role="status">{notice}</p>{/if}
    {#if error}
      <p id={errorId} class="error-banner" role="alert">{error}</p>
    {/if}
  </div>
{/if}

{#if confirming}
  <Modal
    title={confirmLabel}
    onclose={() => {
      if (!busy) confirming = undefined;
    }}
  >
    <form
      onsubmit={(event) => {
        event.preventDefault();
        if (confirming) void changeStatus(confirming);
      }}
    >
      <p class="confirmation">
        「{item.title}」を{confirming === "blocked"
          ? "blockedに変更"
          : "キャンセル"}しますか？
      </p>
      {#if error}<p class="error-banner" role="alert">{error}</p>{/if}
      <button
        class="btn"
        type="button"
        data-autofocus
        disabled={busy}
        onclick={() => (confirming = undefined)}>取りやめ</button
      >
      <button class="btn" type="submit" disabled={busy}>{confirmLabel}</button>
    </form>
  </Modal>
{/if}

<style lang="sass">
  .confirmation
    overflow-wrap: anywhere

  .error-banner
    overflow-wrap: anywhere

  .task-menu
    position: fixed
    right: auto
    min-width: min(180px, calc(100vw - var(--sp-5)))
    max-width: calc(100vw - var(--sp-5))
    max-height: calc(100dvh - var(--sp-5))
    overflow: auto

  // The family card recipe lays its children out in a row; this card reads
  // top to bottom instead, so it stacks and lets the title wrap.
  .stack
    -webkit-touch-callout: none
    user-select: none
    flex-direction: column
    align-items: stretch
    gap: var(--sp-1)

  .stack .name
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
