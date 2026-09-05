<script lang="ts">
  import { untrack } from "svelte";
  import Modal from "./Modal.svelte";
  import type { TaskFields } from "./api";

  let {
    initial,
    title,
    onsave,
    onclose,
  }: {
    initial?: TaskFields;
    title: string;
    onsave: (fields: TaskFields) => Promise<void>;
    onclose: () => void;
  } = $props();
  // A form owns its draft for its lifetime; background card refreshes do not.
  let product = $state(untrack(() => initial?.product_id ?? ""));
  let taskTitle = $state(untrack(() => initial?.title ?? ""));
  let body = $state(untrack(() => initial?.body ?? ""));
  let busy = $state(false);
  let error = $state("");
  let invalid = $derived(!product.trim() || !taskTitle.trim());

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy || invalid) return;
    busy = true;
    error = "";
    try {
      await onsave({
        product_id: product.trim(),
        title: taskTitle.trim(),
        body,
      });
      onclose();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : "保存に失敗しました";
    } finally {
      busy = false;
    }
  }
</script>

<Modal
  {title}
  onclose={() => {
    if (!busy) onclose();
  }}
>
  <form onsubmit={save}>
    <label
      >product<input
        data-autofocus
        bind:value={product}
        disabled={busy}
      /></label
    >
    <label>title<input bind:value={taskTitle} disabled={busy} /></label>
    <label
      >body<textarea rows="8" bind:value={body} disabled={busy}
      ></textarea></label
    >
    {#if invalid}<p class="hint" id="task-form-required">
        product と title を入力してください
      </p>{/if}
    {#if error}<p class="state error" role="alert">{error}</p>{/if}
    <button
      class="btn"
      class:primary={!invalid && !busy}
      type="submit"
      disabled={busy}
      aria-disabled={invalid}
      aria-describedby={invalid ? "task-form-required" : undefined}>保存</button
    >
  </form>
</Modal>

<style lang="sass">
  form, label
    display: flex
    flex-direction: column
    gap: var(--sp-2)

  form
    gap: var(--sp-3)

  label, .hint
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .hint
    margin: 0

  input, textarea
    width: 100%
    min-width: 0
    padding: var(--sp-2)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface)
    color: var(--c-on-surface)
    font-size: var(--fs-lg)
    line-height: 1.6

  textarea
    resize: vertical

  input:focus, textarea:focus
    border-color: var(--c-accent)
</style>
