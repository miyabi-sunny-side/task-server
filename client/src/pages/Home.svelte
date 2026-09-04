<script lang="ts">
  import ControlPanel from "../lib/ControlPanel.svelte";
  import StatusTaskList from "../lib/StatusTaskList.svelte";
  import { startAutoReload } from "../lib/auto-reload";
  import {
    fetchControl,
    fetchTasks,
    type ControlPlane,
    type TaskSummary,
  } from "../lib/api";

  type FetchState = "loading" | "error" | "ready";

  let plane = $state<ControlPlane | undefined>();
  let controlState = $state<FetchState>("loading");
  let items = $state<TaskSummary[]>([]);
  let listState = $state<FetchState>("loading");

  let controlController: AbortController | undefined;
  let listController: AbortController | undefined;
  let controlLoaded = false;
  let listLoaded = false;

  // Whatever the panel is already drawing in a readout, the list below leaves
  // out: one object drawn twice on one screen reads as two.
  let drawnByPanel = $derived([
    ...[
      ...(plane?.pending_reviews ?? []),
      ...(plane?.unreviewed ?? []),
      ...(plane?.mergeable ?? []),
    ].map((item) => item.id),
    // Stuck rows name their task by `task_id`; a `normal` one would otherwise
    // stand in its status group as well.
    ...(plane?.stuck ?? []).map((item) => item.task_id),
  ]);

  function aborted(error: unknown): boolean {
    return error instanceof DOMException && error.name === "AbortError";
  }

  // One request draws the whole panel, jam reasons included: `pending_merges`
  // carries each merge's own `verification`. A second request per stopped merge
  // would be a second thing to fail and a second generation to race, and this
  // region has neither.
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
