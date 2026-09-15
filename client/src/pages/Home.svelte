<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import TaskForm from "../lib/TaskForm.svelte";
  import ControlPanel from "../lib/ControlPanel.svelte";
  import StatusTaskList from "../lib/StatusTaskList.svelte";
  import { startAutoReload } from "../lib/auto-reload";
  import {
    createTask,
    fetchControl,
    fetchTasks,
    fetchExecutionTargets,
    type ControlPlane,
    type TaskSummary,
    type ExecutionTargets,
  } from "../lib/api";

  type FetchState = "loading" | "error" | "ready";

  let {
    creating = false,
    onclose = () => {},
  }: { creating?: boolean; onclose?: () => void } = $props();

  onDestroy(() => onclose());

  let target = $state<string | null | undefined>(undefined);
  let targets = $state<ExecutionTargets>();
  let configError = $state(false);
  let plane = $state<ControlPlane | undefined>();
  let controlState = $state<FetchState>("loading");
  let items = $state<TaskSummary[]>([]);
  let listState = $state<FetchState>("loading");

  let controlController: AbortController | undefined;
  let listController: AbortController | undefined;
  let controlLoaded = false;
  let listLoaded = false;
  let historicalTargets = $derived([
    ...new Set(
      items
        .map((item) => item.execution_target)
        .filter(
          (name): name is string =>
            name != null && !targets?.labels.includes(name),
        ),
    ),
  ]);

  const configController = new AbortController();
  async function loadTargets() {
    configError = false;
    try {
      const loaded = await fetchExecutionTargets(configController.signal);
      if (!configController.signal.aborted) targets = loaded;
    } catch {
      if (!configController.signal.aborted) configError = true;
    }
  }
  onMount(() => {
    void loadTargets();
    return () => configController.abort();
  });

  let visibleItems = $derived(
    items.filter(
      (item) =>
        target === undefined || (item.execution_target ?? null) === target,
    ),
  );
  let visiblePlane = $derived(
    plane && target !== undefined
      ? {
          ...plane,
          stuck: plane.stuck.filter((row) =>
            visibleItems.some((item) => item.id === row.task_id),
          ),
        }
      : plane,
  );

  let drawnByPanel = $derived(
    (visiblePlane?.stuck ?? [])
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
    const controller = new AbortController();
    controlController = controller;
    if (!controlLoaded) {
      controlState = "loading";
    }
    try {
      const loaded = await fetchControl(controller.signal);
      if (controller.signal.aborted) return;
      plane = loaded;
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
    const controller = new AbortController();
    listController = controller;
    if (!listLoaded) {
      listState = "loading";
    }
    try {
      const loaded = await fetchTasks(controller.signal);
      if (controller.signal.aborted) return;
      items = loaded;
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

  async function onupdated(task: TaskSummary) {
    listController?.abort();
    controlController?.abort();
    items = items.map((item) =>
      item.id === task.id ? { ...item, ...task } : item,
    );
    if (plane)
      plane = {
        ...plane,
        stuck: plane.stuck.filter((row) => row.task_id !== task.id),
      };
    await loadBoth();
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
    <div class="target-filter">
      <label for="execution-target-filter">実行先で絞り込み</label>
      <select id="execution-target-filter" class="btn" bind:value={target}>
        <option value={undefined}>すべて</option>
        {#each targets?.labels ?? [] as name}
          <option value={name}>{name}</option>
        {/each}
        {#if historicalTargets.length}
          <optgroup label="記録済みの実行先">
            {#each historicalTargets as name}
              <option value={name}
                >{name}{targets ? "（現在の設定にありません）" : ""}</option
              >
            {/each}
          </optgroup>
        {/if}
        {#if items.some((item) => item.execution_target == null)}
          <option value={null}>未設定</option>
        {/if}
      </select>
      {#if configError}
        <span role="alert">実行先の設定を読み込めませんでした</span>
        <button class="btn" type="button" onclick={() => void loadTargets()}
          >実行先を再読み込み</button
        >
      {:else if targets?.labels.length === 0}
        <span>実行先が設定されていません</span>
      {/if}
    </div>
  </div>
  {#if creating}
    <TaskForm
      title="新規タスク"
      {onclose}
      onsave={async (fields) => {
        await createTask(fields);
        await loadList();
      }}
    />
  {/if}
  <ControlPanel
    fetchState={controlState}
    plane={visiblePlane}
    tasks={visibleItems}
    {onupdated}
    onretry={() => void loadControl()}
  />
  <StatusTaskList
    fetchState={listState}
    items={visibleItems}
    {onupdated}
    drawnElsewhere={drawnByPanel}
    onretry={() => void loadList()}
  />
</div>

<style lang="sass">
  .actions
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-3)
    margin-bottom: var(--sp-4)
  .target-filter
    display: flex
    align-items: center
    flex-wrap: wrap
    gap: var(--sp-2)
    font-size: var(--fs-xs)
    color: var(--c-muted)
    min-width: 0
    flex: 1

  select
    max-width: 100%
    min-width: 0
    flex: 1
</style>
