<script lang="ts">
  import { tick } from "svelte";
  import {
    ApiError,
    archiveIdea,
    fetchIdea,
    promoteIdea,
    updateIdea,
    type Idea,
  } from "../lib/api";
  import { startAutoReload } from "../lib/auto-reload";
  import Modal from "../lib/Modal.svelte";
  import TaskForm from "../lib/TaskForm.svelte";
  import { navigate } from "../lib/router.svelte";

  let { id }: { id: string } = $props();

  let idea = $state<Idea | undefined>();
  let detailState = $state<"loading" | "error" | "success">("loading");
  let controller: AbortController | undefined;
  let loadedId: string | undefined;

  // The draft belongs to the editor for its whole life: background reloads
  // replace `idea`, never these fields.
  let editing = $state(false);
  let baseRevision = 0;
  let draftTitle = $state("");
  let draftProduct = $state("");
  let draftBody = $state("");
  // The newer record a save collided with, shown until the user chooses.
  let conflict = $state<Idea | undefined>();
  let saveError = $state("");
  let busy = $state(false);

  let archiving = $state(false);
  let archiveError = $state("");
  let promoting = $state(false);
  let editButton = $state<HTMLButtonElement | undefined>();

  async function load(currentId: string) {
    controller?.abort();
    controller = new AbortController();
    try {
      idea = await fetchIdea(currentId, controller.signal);
      detailState = "success";
      loadedId = currentId;
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      if (loadedId !== currentId) {
        detailState = "error";
      }
    }
  }

  function startEdit() {
    if (!idea) return;
    baseRevision = idea.revision;
    draftTitle = idea.title;
    draftProduct = idea.product_id ?? "";
    draftBody = idea.body;
    conflict = undefined;
    saveError = "";
    editing = true;
  }

  async function finishEdit() {
    editing = false;
    conflict = undefined;
    saveError = "";
    await tick();
    editButton?.focus();
  }

  let invalid = $derived(draftTitle.trim() ? "" : "title を入力してください");

  async function save(revision: number) {
    if (busy || invalid) return;
    busy = true;
    saveError = "";
    try {
      idea = await updateIdea(id, revision, {
        title: draftTitle.trim(),
        product_id: draftProduct.trim() || null,
        body: draftBody,
      });
      await finishEdit();
    } catch (error) {
      if (error instanceof ApiError && error.status === 409) {
        try {
          conflict = await fetchIdea(id);
          idea = conflict;
        } catch {
          saveError = "最新の内容を読み込めませんでした";
        }
      } else {
        saveError =
          error instanceof Error ? error.message : "保存に失敗しました";
      }
    } finally {
      busy = false;
    }
  }

  async function discard() {
    if (conflict) idea = conflict;
    await finishEdit();
  }

  async function confirmArchive() {
    if (busy) return;
    busy = true;
    archiveError = "";
    try {
      idea = await archiveIdea(id);
      archiving = false;
    } catch (error) {
      archiveError =
        error instanceof Error ? error.message : "アーカイブに失敗しました";
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    if (loadedId !== id) {
      detailState = "loading";
      editing = false;
    }
    void load(id);
    const stopAutoReload = startAutoReload(() => void load(id));
    return () => {
      stopAutoReload();
      controller?.abort();
    };
  });
</script>

<div class="sub-header">
  <h1 class="sub-title">{idea ? idea.title : "アイデア"}</h1>
</div>

<div class="content">
  {#if detailState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if detailState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load(id)}>
        再試行
      </button>
    </div>
  {:else if idea}
    <div class="meta">
      {#if idea.product_id}<p class="product">{idea.product_id}</p>{/if}
      <p class="caption">
        更新 {idea.updated_at} ・ 作成 {idea.created_at}
      </p>
      {#if idea.archived || idea.task_id}
        <p class="badges">
          {#if idea.archived}<span class="badge">アーカイブ済み</span>{/if}
          {#if idea.task_id}
            <span class="badge">タスク化済み</span>
            <a href={`/tasks/${encodeURIComponent(idea.task_id)}`}
              >タスクを開く</a
            >
          {/if}
        </p>
      {/if}
    </div>

    {#if editing}
      {#if conflict}
        <section class="conflict">
          <p class="error-banner" role="alert">
            {conflict.archived
              ? "このアイデアはアーカイブされたため保存できません。下書きは下に残っています。"
              : "他の更新が保存されています。最新の内容を確認し、下書きで上書きするか破棄してください。"}
          </p>
          <p class="caption">最新（更新 {conflict.updated_at}）</p>
          <p class="latest-title">{conflict.title}</p>
          <p class="text latest-body">{conflict.body}</p>
        </section>
      {/if}
      <form
        class="editor"
        id="idea-editor"
        onsubmit={(event) => {
          event.preventDefault();
          void save(baseRevision);
        }}
      >
        <label
          >title<input
            bind:value={draftTitle}
            disabled={busy}
            autocomplete="off"
          /></label
        >
        <label
          >product<input
            bind:value={draftProduct}
            disabled={busy}
            autocomplete="off"
            placeholder="任意"
          /></label
        >
        <label
          >body<textarea rows="14" bind:value={draftBody} disabled={busy}
          ></textarea></label
        >
      </form>
    {:else if idea.body.trim()}
      <p class="text">{idea.body}</p>
    {:else}
      <p class="note">本文はまだありません</p>
    {/if}

    {#if editing}
      <footer class="action-footer" aria-label="アイデアの編集">
        {#if saveError}<p class="error-banner" role="alert">{saveError}</p>{/if}
        {#if invalid}<p class="note" id="idea-required">{invalid}</p>{/if}
        <div class="actions">
          {#if conflict}
            {#if !conflict.archived}
              <button
                class="btn"
                class:primary={!busy && !invalid}
                type="button"
                disabled={busy}
                aria-disabled={!!invalid}
                aria-describedby={invalid ? "idea-required" : undefined}
                onclick={() => conflict && void save(conflict.revision)}
                >この内容で上書き保存</button
              >
            {/if}
            <button
              class="btn"
              type="button"
              disabled={busy}
              onclick={() => void discard()}>下書きを破棄</button
            >
          {:else}
            <button
              class="btn"
              class:primary={!busy && !invalid}
              type="submit"
              form="idea-editor"
              disabled={busy}
              aria-disabled={!!invalid}
              aria-describedby={invalid ? "idea-required" : undefined}
              >保存</button
            >
            <button
              class="btn"
              type="button"
              disabled={busy}
              onclick={() => void finishEdit()}>キャンセル</button
            >
          {/if}
        </div>
      </footer>
    {:else if !idea.archived}
      <footer class="action-footer" aria-label="アイデアの操作">
        <div class="actions">
          <button
            class="btn"
            type="button"
            bind:this={editButton}
            onclick={startEdit}>編集</button
          >
          {#if !idea.task_id}
            <button class="btn" type="button" onclick={() => (promoting = true)}
              >タスク化</button
            >
          {/if}
          <button
            class="btn"
            type="button"
            onclick={() => {
              archiveError = "";
              archiving = true;
            }}>アーカイブ</button
          >
        </div>
      </footer>
    {/if}
  {/if}
</div>

{#if archiving && idea}
  <Modal
    title="アイデアをアーカイブ"
    onclose={() => {
      if (!busy) archiving = false;
    }}
  >
    <p class="dialog-text">
      「{idea.title}」を一覧から外します。本文はアーカイブから読めます。
    </p>
    {#if archiveError}<p class="error-banner" role="alert">
        {archiveError}
      </p>{/if}
    <div class="actions">
      <button
        class="btn"
        class:primary={!busy}
        type="button"
        data-autofocus
        disabled={busy}
        onclick={() => void confirmArchive()}>アーカイブ</button
      >
      <button
        class="btn"
        type="button"
        disabled={busy}
        onclick={() => (archiving = false)}>取りやめ</button
      >
    </div>
  </Modal>
{/if}

{#if promoting && idea}
  <TaskForm
    title="アイデアをタスク化"
    submitLabel="タスクを作成"
    prefill={{
      product_id: idea.product_id ?? "",
      title: idea.title,
      body: idea.body,
    }}
    onclose={() => (promoting = false)}
    onsave={async (fields) => {
      const { task } = await promoteIdea(id, fields);
      navigate(`/tasks/${encodeURIComponent(task.id)}`);
    }}
  />
{/if}

<style lang="sass">
  .content
    display: flex
    flex-direction: column
    min-height: calc(100dvh - var(--header-h) - var(--subheader-h))
    padding-bottom: 0

  .sub-header
    position: sticky
    top: var(--header-h)
    z-index: 9
    display: flex
    align-items: center
    height: var(--subheader-h)
    padding: 0 var(--sp-3)
    background: var(--c-wash-raised)
    border-bottom: 1px solid var(--c-border)

  .sub-title
    margin: 0
    overflow: hidden
    text-overflow: ellipsis
    white-space: nowrap
    font-size: var(--fs-md)
    font-weight: 500
    line-height: 1.2

  .meta
    display: flex
    flex-direction: column
    gap: var(--sp-1)
    margin: 0 0 var(--sp-3)

  .product
    margin: 0
    font-size: var(--fs-sm)
    line-height: 1.4
    overflow-wrap: anywhere

  .caption
    margin: 0
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .badges
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-2)
    margin: 0
    font-size: var(--fs-sm)

  .text
    margin: 0 0 var(--sp-4)
    white-space: pre-line
    overflow-wrap: anywhere

  .note
    margin: 0 0 var(--sp-4)

  .conflict
    display: flex
    flex-direction: column
    gap: var(--sp-2)
    margin: 0 0 var(--sp-4)
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)

  .latest-title
    margin: 0
    font-size: var(--fs-md)
    font-weight: 500
    overflow-wrap: anywhere

  .latest-body
    margin: 0
    font-size: var(--fs-sm)
    line-height: 1.5

  .editor, label
    display: flex
    flex-direction: column
    gap: var(--sp-2)

  .editor
    gap: var(--sp-3)
    margin: 0 0 var(--sp-4)

  label
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

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

  // DESIGN.md, Detail action footer: the same sticky bottom recipe.
  .action-footer
    position: sticky
    bottom: 0
    z-index: 8
    display: flex
    flex-direction: column
    gap: var(--sp-2)
    margin: auto calc(-1 * var(--sp-3)) 0
    padding: var(--sp-2) var(--sp-3)
    border-top: 1px solid var(--c-border)
    background: var(--c-surface-raised)

  .actions
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)

  .dialog-text
    margin: 0 0 var(--sp-3)
    font-size: var(--fs-sm)
    overflow-wrap: anywhere
</style>
