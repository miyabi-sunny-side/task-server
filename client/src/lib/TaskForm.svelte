<script lang="ts">
  import { onMount, untrack } from "svelte";
  import Modal from "./Modal.svelte";
  import {
    fetchExecutionTargets,
    type ExecutionTargets,
    type TaskFields,
  } from "./api";

  let {
    initial,
    prefill,
    submitLabel = "保存",
    title,
    onsave,
    onclose,
  }: {
    initial?: TaskFields;
    // Starting values for a new task (e.g. promoting an idea); unlike
    // `initial`, the form still creates, so the target default applies.
    prefill?: TaskFields;
    submitLabel?: string;
    title: string;
    onsave: (fields: TaskFields) => Promise<void>;
    onclose: () => void;
  } = $props();
  // A form owns its draft for its lifetime; background card refreshes do not.
  const start = untrack(() => initial ?? prefill);
  let product = $state(start?.product_id ?? "");
  let taskTitle = $state(start?.title ?? "");
  const originalTarget = untrack(() => initial?.execution_target ?? undefined);
  let target = $state<string | undefined>(originalTarget);
  let targets = $state<ExecutionTargets>();
  let configError = $state(false);
  let body = $state(start?.body ?? "");
  let busy = $state(false);
  let error = $state("");
  let invalid = $derived(
    !product.trim() || !taskTitle.trim()
      ? "product と title を入力してください"
      : (!initial || target !== originalTarget) &&
          (!target || !targets?.labels.includes(target))
        ? "実行先を選択してください"
        : "",
  );

  const controller = new AbortController();
  async function loadTargets() {
    configError = false;
    try {
      const loaded = await fetchExecutionTargets(controller.signal);
      if (controller.signal.aborted) return;
      targets = loaded;
      if (!initial && target === undefined)
        target = loaded.default ?? undefined;
    } catch {
      if (!controller.signal.aborted) configError = true;
    }
  }
  onMount(() => {
    void loadTargets();
    return () => controller.abort();
  });

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
        ...(!initial || target !== originalTarget
          ? { execution_target: target }
          : {}),
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
    <div class="field">
      <label for="task-execution-target">実行先</label>
      <select
        id="task-execution-target"
        bind:value={target}
        disabled={busy || !targets?.labels.length}
      >
        <option value={undefined} disabled
          >{initial ? "未設定" : "実行先を選択"}</option
        >
        {#if originalTarget && !targets?.labels.includes(originalTarget)}
          <option value={originalTarget}
            >{originalTarget}{targets
              ? "（現在の設定にありません）"
              : ""}</option
          >
        {/if}
        {#each targets?.labels ?? [] as name}
          <option value={name}>{name}</option>
        {/each}
      </select>
      {#if configError}
        <p class="hint" role="alert">実行先の設定を読み込めませんでした</p>
        <button
          class="btn"
          type="button"
          disabled={busy}
          onclick={() => void loadTargets()}>実行先を再読み込み</button
        >
      {:else if !targets}
        <p class="hint" role="status">実行先を読み込み中…</p>
      {:else if targets.labels.length === 0}
        <p class="hint">実行先が設定されていません</p>
      {/if}
    </div>
    <label>title<input bind:value={taskTitle} disabled={busy} /></label>
    <label
      >body<textarea rows="8" bind:value={body} disabled={busy}
      ></textarea></label
    >
    {#if invalid}<p class="hint" id="task-form-required">
        {invalid}
      </p>{/if}
    {#if error}<p class="state error" role="alert">{error}</p>{/if}
    <button
      class="btn"
      class:primary={!invalid && !busy}
      type="submit"
      disabled={busy}
      aria-disabled={!!invalid}
      aria-describedby={invalid ? "task-form-required" : undefined}
      >{submitLabel}</button
    >
  </form>
</Modal>

<style lang="sass">
  form, label, .field
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

  input, textarea, select
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

  input:focus, textarea:focus, select:focus
    border-color: var(--c-accent)
</style>
