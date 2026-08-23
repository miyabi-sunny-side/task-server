<script lang="ts">
  import ControlPanel, { type ActionResult } from "../lib/ControlPanel.svelte";
  import ReleaseModal from "../lib/ReleaseModal.svelte";
  import StatusTaskList from "../lib/StatusTaskList.svelte";
  import {
    fetchControl,
    fetchTasks,
    postRelease,
    type ControlPlane,
    type TaskSummary,
  } from "../lib/api";

  type FetchState = "loading" | "error" | "ready";

  let plane = $state<ControlPlane | undefined>();
  let controlState = $state<FetchState>("loading");
  let items = $state<TaskSummary[]>([]);
  let listState = $state<FetchState>("loading");

  let busy = $state(false);
  let result = $state<ActionResult>({ kind: "none", message: "" });
  let modalOpen = $state(false);
  let releaseError = $state("");

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

  function reason(error: unknown): string {
    return error instanceof Error && error.message !== ""
      ? error.message
      : "原因不明のエラー";
  }

  // One request draws the whole panel, jam reasons included: `pending_merges`
  // carries each merge's own `verification`. A second request per blocked head
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

  function onrelease() {
    if ((plane?.releasable ?? []).length === 0) {
      return;
    }
    releaseError = "";
    modalOpen = true;
  }

  function closeModal() {
    modalOpen = false;
    // Focus returns to the control that opened the modal (DESIGN.md, Modals).
    queueMicrotask(() => document.getElementById("control-release")?.focus());
  }

  async function onconfirmRelease(productId: string, tag: string) {
    busy = true;
    releaseError = "";
    try {
      const released = await postRelease(productId, tag);
      closeModal();
      result = {
        kind: "success",
        message: `${released.product_id} を ${released.tag} で release しました (${released.released.length} 件)`,
      };
      busy = false;
      await loadBoth();
    } catch (error) {
      // A refused release is corrected where the tag was typed.
      releaseError = reason(error);
      busy = false;
    }
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
    {busy}
    {result}
    {onrelease}
    onretry={() => void loadControl()}
  />
  <StatusTaskList
    fetchState={listState}
    {items}
    drawnElsewhere={drawnByPanel}
    onretry={() => void loadList()}
  />
</div>

{#if modalOpen}
  <ReleaseModal
    releasable={plane?.releasable ?? []}
    {busy}
    error={releaseError}
    onclose={closeModal}
    onconfirm={(productId, tag) => void onconfirmRelease(productId, tag)}
  />
{/if}
