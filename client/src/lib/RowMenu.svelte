<script lang="ts">
  // A card link that also opens a context menu (DESIGN.md, Task context
  // menu): the row owns opening, placement, keyboard and dismissal; the
  // caller owns the items and what they do.
  import { onMount, tick, untrack, type Snippet } from "svelte";

  let {
    href,
    id,
    label,
    closeLabel,
    open = $bindable(false),
    row = $bindable(),
    error = "",
    errorId,
    errorInDialog = false,
    notice = "",
    children,
    items,
  }: {
    href: string;
    id: string;
    label: string;
    closeLabel: string;
    open?: boolean;
    row?: HTMLAnchorElement;
    error?: string;
    errorId: string;
    // A confirmation dialog shows the error itself; the row still refers to it.
    errorInDialog?: boolean;
    notice?: string;
    children: Snippet;
    items: Snippet;
  } = $props();
  let menu = $state<HTMLElement>();
  let x = $state(0);
  let y = $state(0);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let origin: { x: number; y: number } | undefined;
  let held = false;
  let anchorTop = 0;

  function cancelPress() {
    clearTimeout(timer);
    origin = undefined;
  }

  // Closing returns focus to the row; callers that remove the row set
  // `open` to false themselves instead.
  export function close() {
    if (!open) return;
    open = false;
    row?.focus({ preventScroll: true });
  }

  function placeMenu() {
    if (!menu || !row) return;
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

  // A message appearing inside an open menu changes its height.
  $effect(() => {
    void error;
    void notice;
    untrack(() => {
      if (open) placeMenu();
    });
  });

  async function openMenu() {
    cancelPress();
    if (open || !row) return;
    open = true;
    anchorTop = row.getBoundingClientRect().top;
    await tick();
    if (!open || !menu) return;
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
    if (!open) return;
    if (event.key === "Escape" || event.key === "Tab") {
      if (event.key === "Escape") event.preventDefault();
      close();
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
        open &&
        !(event.target instanceof Node && menu?.contains(event.target)) &&
        row?.getBoundingClientRect().top !== anchorTop
      )
        close();
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

<svelte:window onkeydown={menuKeydown} onresize={close} />

<a
  class="card row"
  {href}
  {id}
  bind:this={row}
  aria-haspopup="menu"
  aria-expanded={open}
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
  {@render children()}
</a>

{#if notice && !open}<p role="status">{notice}</p>{/if}
{#if error && !open && !errorInDialog}
  <p id={errorId} class="error-banner" role="alert">{error}</p>
{/if}
{#if open}
  <button
    class="menu-overlay"
    type="button"
    tabindex="-1"
    aria-label={closeLabel}
    onclick={close}
  ></button>
  <div
    class="menu row-menu"
    bind:this={menu}
    style:left={`${x}px`}
    style:top={`${y}px`}
  >
    <div role="menu" aria-label={label}>
      {@render items()}
    </div>
    {#if notice}<p role="status">{notice}</p>{/if}
    {#if error}
      <p id={errorId} class="error-banner" role="alert">{error}</p>
    {/if}
  </div>
{/if}

<style lang="sass">
  .row-menu > p
    padding-inline: var(--sp-3)

  .error-banner
    overflow-wrap: anywhere

  .row-menu
    position: fixed
    right: auto
    min-width: min(180px, calc(100vw - var(--sp-5)))
    max-width: calc(100vw - var(--sp-5))
    max-height: calc(100dvh - var(--sp-5))
    overflow: auto

  // The family card recipe lays its children out in a row; this card reads
  // top to bottom instead, so it stacks and lets the title wrap. Row text
  // is not selectable so a touch hold belongs to the menu.
  .row
    -webkit-touch-callout: none
    user-select: none
    flex-direction: column
    align-items: stretch
    gap: var(--sp-1)
</style>
