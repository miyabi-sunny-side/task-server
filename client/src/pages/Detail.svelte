<script lang="ts">
  import { fetchTask, postTaskStatus, type TaskCard as Task } from "../lib/api";
  import TaskCard from "../lib/TaskCard.svelte";

  let { id }: { id: string } = $props();

  let task = $state<Task | undefined>();
  let detailState = $state<"loading" | "error" | "success">("loading");
  let busy = $state(false);
  let actionError = $state("");

  let controller: AbortController | undefined;

  async function load(currentId: string) {
    controller?.abort();
    controller = new AbortController();
    try {
      task = await fetchTask(currentId, controller.signal);
      detailState = "success";
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      detailState = "error";
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
    } catch {
      actionError = "操作に失敗しました";
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    void load(id);
    return () => controller?.abort();
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
    <TaskCard {task} {busy} error={actionError} {ontransition} />
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
