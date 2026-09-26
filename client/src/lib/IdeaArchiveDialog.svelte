<script lang="ts">
  // The one archive confirmation, from an idea's page or its list row.
  // While the request runs, nothing dismisses it and the buttons keep focus.
  import Modal from "./Modal.svelte";

  let {
    title,
    busy,
    error,
    errorId,
    onconfirm,
    onclose,
  }: {
    title: string;
    busy: boolean;
    error: string;
    errorId?: string;
    onconfirm: () => void;
    onclose: () => void;
  } = $props();

  function close() {
    if (!busy) onclose();
  }
</script>

<Modal title="アイデアをアーカイブ" onclose={close}>
  <p class="dialog-text">
    「{title}」を一覧から外します。本文はアーカイブから読めます。
  </p>
  {#if error}<p id={errorId} class="error-banner" role="alert">
      {error}
    </p>{/if}
  <div class="actions">
    <button
      class="btn"
      class:primary={!busy}
      type="button"
      data-autofocus
      aria-disabled={busy}
      onclick={() => {
        if (!busy) onconfirm();
      }}>アーカイブ</button
    >
    <button class="btn" type="button" aria-disabled={busy} onclick={close}
      >取りやめ</button
    >
  </div>
</Modal>

<style lang="sass">
  .actions
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)

  .dialog-text
    margin: 0 0 var(--sp-3)
    font-size: var(--fs-sm)
    overflow-wrap: anywhere

  .error-banner
    margin: 0 0 var(--sp-3)
    overflow-wrap: anywhere
</style>
