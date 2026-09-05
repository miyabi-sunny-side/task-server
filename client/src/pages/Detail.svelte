<script lang="ts">
  import {
    fetchTask,
    postTaskStatus,
    updateTask,
    type TaskCard as Task,
  } from "../lib/api";
  import TaskForm from "../lib/TaskForm.svelte";
  import RunHistory from "../lib/RunHistory.svelte";
  import TaskCard from "../lib/TaskCard.svelte";
  import { startAutoReload } from "../lib/auto-reload";

  let { id }: { id: string } = $props();

  let task = $state<Task | undefined>();
  let detailState = $state<"loading" | "error" | "success">("loading");
  let editing = $state(false);
  let busy = $state(false);
  let actionError = $state("");
  let selectedReport = $state<number>();

  let controller: AbortController | undefined;
  // The id whose card is currently drawn, so a background reload's failure
  // never displaces it — only a fresh id without a card yet falls back to
  // the error state.
  let loadedId: string | undefined;

  async function load(currentId: string) {
    controller?.abort();
    controller = new AbortController();
    try {
      task = await fetchTask(currentId, controller.signal);
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

  async function ontransition(status: string) {
    if (!task) {
      return;
    }
    busy = true;
    actionError = "";
    try {
      await postTaskStatus(task.id, status);
      await load(task.id);
    } catch (error) {
      actionError =
        error instanceof Error ? error.message : "操作に失敗しました";
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    if (loadedId !== id) {
      detailState = "loading";
      selectedReport = undefined;
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
  <h1 class="sub-title">{task ? task.title : "詳細"}</h1>
</div>

<div class="content">
  {#if detailState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if detailState === "error"}
    <p class="state error">読み込みに失敗しました</p>
  {:else if task}
    <TaskCard
      {task}
      {busy}
      error={actionError}
      {ontransition}
      onedit={() => (editing = true)}
      onreport={(id) => (selectedReport = id)}
    />
    {#if editing}
      <TaskForm
        title="タスクを編集"
        initial={task}
        onclose={() => (editing = false)}
        onsave={async (fields) => {
          if (task) {
            await updateTask(task.id, fields);
            await load(task.id);
          }
        }}
      />
    {/if}
    {#key task.id}<RunHistory
        taskId={task.id}
        selectedReport={selectedReport ??
          (task.status === "blocked" ? task.report_id : undefined)}
      />{/key}
  {/if}
</div>

<style lang="sass">
  .sub-header
    position: sticky
    top: var(--header-h)
    z-index: 9
    display: flex
    align-items: center
    gap: var(--sp-2)
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
</style>
