<script lang="ts">
  import Modal from "./Modal.svelte";
  import type { Releasable } from "./api";

  let {
    releasable,
    busy = false,
    error = "",
    onclose,
    onconfirm,
  }: {
    releasable: Releasable[];
    busy?: boolean;
    error?: string;
    onclose: () => void;
    onconfirm: (productId: string, tag: string) => void;
  } = $props();

  // Until one is picked, the first product stands selected, so the tag field
  // below is always about something (DESIGN.md, Release modal).
  let picked = $state("");
  let selected = $derived(picked || (releasable[0]?.product_id ?? ""));
  let tag = $state("");

  let blank = $derived(tag.trim() === "");
  let only = $derived(releasable.length === 1 ? releasable[0] : undefined);

  function confirm() {
    if (blank || busy) {
      return;
    }
    onconfirm(selected, tag.trim());
  }
</script>

<Modal title="release" {onclose}>
  <div class="body">
    {#if only}
      <p class="note">
        {only.product_id} — merged {only.task_count} 件
      </p>
    {:else}
      <div class="products" role="radiogroup" aria-label="product">
        {#each releasable as item (item.product_id)}
          <button
            class="product-row"
            class:selected={selected === item.product_id}
            type="button"
            role="radio"
            aria-checked={selected === item.product_id}
            onclick={() => (picked = item.product_id)}
          >
            <span class="product-id">{item.product_id}</span>
            <span class="pill" data-count>{item.task_count}</span>
          </button>
        {/each}
      </div>
    {/if}

    <div class="field">
      <label class="note" for="release-tag">tag</label>
      <input
        id="release-tag"
        class="input"
        type="text"
        value={tag}
        data-autofocus
        oninput={(event) => (tag = event.currentTarget.value)}
      />
    </div>

    {#if error}
      <p class="error-banner" role="alert">{error}</p>
    {/if}

    <div class="actions">
      <button class="btn" type="button" disabled={busy} onclick={onclose}>
        キャンセル
      </button>
      <button
        class="btn primary"
        type="button"
        aria-disabled={blank ? "true" : undefined}
        aria-describedby="release-tag-note"
        disabled={busy}
        onclick={confirm}
      >
        release する
      </button>
    </div>
    {#if blank}
      <p class="note" id="release-tag-note">tag は必須です</p>
    {/if}
  </div>
</Modal>

<style lang="sass">
  .body
    display: flex
    flex-direction: column
    gap: var(--sp-3)

  .products
    display: flex
    flex-direction: column
    gap: var(--sp-2)

  .product-row
    display: flex
    align-items: center
    justify-content: space-between
    gap: var(--sp-2)
    min-height: 36px
    padding: var(--sp-2) var(--sp-3)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface-raised)
    color: var(--c-on-surface)
    cursor: pointer

    &:hover
      background: var(--c-hover-1)

    &.selected
      border-color: var(--c-accent)
      background: var(--c-accent-subtle)

  .product-id
    font-size: var(--fs-md)
    font-weight: 500
    line-height: 1.2

  .field
    display: flex
    flex-direction: column
    gap: var(--sp-1)

  .input
    padding: var(--sp-2)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface)
    color: var(--c-on-surface)
    font-family: inherit
    font-size: var(--fs-lg)
    line-height: 1.6

    &:focus
      border-color: var(--c-accent)

  .actions
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
</style>
