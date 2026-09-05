<script lang="ts">
  import TaskForm from "../lib/TaskForm.svelte";
  import ControlPanel from "../lib/ControlPanel.svelte";
  import StatusTaskList from "../lib/StatusTaskList.svelte";
  import { startAutoReload } from "../lib/auto-reload";
  import {
    createTask,
    fetchControl,
    fetchTasks,
    type ControlPlane,
    type TaskSummary,
  } from "../lib/api";

  type FetchState = "loading" | "error" | "ready";

  let creating = $state(false);
  let plane = $state<ControlPlane | undefined>();
  let controlState = $state<FetchState>("loading");
  let items = $state<TaskSummary[]>([]);
  let listState = $state<FetchState>("loading");

  let controlController: AbortController | undefined;
  let listController: AbortController | undefined;
  let controlLoaded = false;
  let listLoaded = false;

  let drawnByPanel = $derived(
    (plane?.stuck ?? [])
      .filter(
        (row) => row.reason === "blocked" || row.reason === "lease-expired",
      )
      .map((row) => row.task_id),
  );

  function aborted(error: unknown): boolean {
    return error instanceof DOMException && error.name === "AbortError";
  }

  async function loadControl() {
    controlController?.abort();
    controlController = new AbortController();
    if (!controlLoaded) {
      controlState = "loading";
    }
    try {
      plane = await fetchControl(controlController.signal);
      controlState = "ready";
      controlLoaded = true;
    } catch (error) {
      // A background reload that fails leaves an already-drawn panel alone;
      // only a first load without data yet to show falls back to the error
      // state (DESIGN.md, Result line).
      if (!aborted(error) && !controlLoaded) {
        controlState = "error";
      }
    }
  }

  async function loadList() {
    listController?.abort();
    listController = new AbortController();
    if (!listLoaded) {
      listState = "loading";
    }
    try {
      items = await fetchTasks(listController.signal);
      listState = "ready";
      listLoaded = true;
    } catch (error) {
      if (!aborted(error) && !listLoaded) {
        listState = "error";
      }
    }
  }

  // Two requests, two regions: one failing never hides the other.
  function loadBoth(): Promise<unknown> {
    return Promise.all([loadControl(), loadList()]);
  }

  $effect(() => {
    void loadBoth();
    const stopAutoReload = startAutoReload(() => void loadBoth());
    return () => {
      stopAutoReload();
      controlController?.abort();
      listController?.abort();
    };
  });
</script>

<div class="content">
  <div class="actions">
    <button class="btn primary" type="button" onclick={() => (creating = true)}
      >新規タスク</button
    >
  </div>
  {#if creating}
    <TaskForm
      title="新規タスク"
      onclose={() => (creating = false)}
      onsave={async (fields) => {
        await createTask(fields);
        await loadList();
      }}
    />
  {/if}
  <ControlPanel
    fetchState={controlState}
    {plane}
    tasks={items}
    onretry={() => void loadControl()}
  />
  <StatusTaskList
    fetchState={listState}
    {items}
    drawnElsewhere={drawnByPanel}
    onretry={() => void loadList()}
  />
</div>

<style lang="sass">
  .actions
    margin-bottom: var(--sp-4)
</style>
