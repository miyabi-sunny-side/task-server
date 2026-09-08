<script lang="ts">
  import Icon from "./Icon.svelte";
  import ThemeModal from "./ThemeModal.svelte";
  import { router } from "./router.svelte";

  let menuOpen = $state(false);
  let themeOpen = $state(false);
  let menuButton = $state<HTMLButtonElement | undefined>();

  let onClosed = $derived(router.index === 2);

  function closeMenu() {
    menuOpen = false;
    menuButton?.focus();
  }

  function openTheme() {
    menuOpen = false;
    themeOpen = true;
  }

  function closeTheme() {
    themeOpen = false;
    menuButton?.focus();
  }

  function onkeydown(event: KeyboardEvent) {
    if (menuOpen && event.key === "Escape") {
      event.preventDefault();
      closeMenu();
    }
  }
</script>

<svelte:window {onkeydown} />

<header>
  <div class="nav">
    <a class="title" href="/">Task Server</a>
    <a
      class="done-link"
      class:selected={onClosed}
      href="/closed"
      aria-current={onClosed ? "page" : undefined}
    >
      closed
    </a>
  </div>
  <div class="menu-wrapper">
    <button
      class="icon-btn"
      type="button"
      aria-label="メニュー"
      aria-expanded={menuOpen}
      bind:this={menuButton}
      onclick={() => (menuOpen = !menuOpen)}
    >
      <Icon name="menu" />
    </button>
    {#if menuOpen}
      <button
        class="menu-overlay"
        type="button"
        tabindex="-1"
        aria-label="メニューを閉じる"
        onclick={closeMenu}
      ></button>
      <nav class="menu">
        <button class="menu-item" type="button" onclick={openTheme}>
          テーマ設定
        </button>
        <a
          class="menu-item"
          href="/products"
          aria-current={router.index === 3 ? "page" : undefined}
          onclick={closeMenu}>プロダクト一覧</a
        >
      </nav>
    {/if}
  </div>
</header>

{#if themeOpen}
  <ThemeModal onclose={closeTheme} />
{/if}

<style lang="sass">
  header
    position: sticky
    top: 0
    z-index: 10
    display: flex
    align-items: center
    justify-content: space-between
    height: var(--header-h)
    padding: 0 var(--sp-3)
    background: var(--c-wash-base)
    border-bottom: 1px solid var(--c-border)

  .nav
    display: flex
    align-items: center
    gap: var(--sp-2)

  .title
    font-size: var(--fs-md)
    font-weight: 500
    color: var(--c-on-surface)
    text-decoration: none

  .done-link
    display: inline-flex
    align-items: center
    min-height: 36px
    padding: var(--sp-1) var(--sp-2)
    border-radius: var(--radius-sm)
    font-size: var(--fs-md)
    font-weight: 500
    color: var(--c-on-surface)
    text-decoration: none

  .done-link:hover
    background: var(--c-hover-1)

  .done-link.selected
    background: var(--c-accent-subtle)
    color: var(--c-accent)

  .menu-wrapper
    position: relative
    display: flex
    align-items: center
    align-self: stretch

</style>
