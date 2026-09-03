<script lang="ts">
  import ControlPanel from "../lib/ControlPanel.svelte";
  import StatusTaskList from "../lib/StatusTaskList.svelte";
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
  let drawnByPanel = $derived(
    [
      ...(plane?.pending_reviews ?? []),
      ...(plane?.unreviewed ?? []),
      ...(plane?.mergeable ?? []),
    ].map((item) => item.id),
  );

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
      if (!aborted(error)) {
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
      if (!aborted(error)) {
        listState = "error";
      }
    }
  }

  // Two requests, two regions: one failing never hides the other.
  function loadBoth(): Promise<unknown> {
    return Promise.all([loadControl(), loadList()]);
  }

  function onVisibilityChange() {
    if (document.visibilityState === "visible") {
      void loadBoth();
    }
  }

  $effect(() => {
    void loadBoth();
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      controlController?.abort();
      listController?.abort();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  });
</script>

<div class="content">
  <ControlPanel
    fetchState={controlState}
    {plane}
    onretry={() => void loadControl()}
  />
  <StatusTaskList
    fetchState={listState}
    {items}
    drawnElsewhere={drawnByPanel}
    onretry={() => void loadList()}
  />
</div>
