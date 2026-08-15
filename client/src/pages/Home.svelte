<script lang="ts">
  import ControlPanel, { type ActionResult } from "../lib/ControlPanel.svelte";
  import ReleaseModal from "../lib/ReleaseModal.svelte";
  import StatusTaskList from "../lib/StatusTaskList.svelte";
  import {
    fetchControl,
    fetchTasks,
    postMerge,
    postRelease,
    type ControlPlane,
    type TaskSummary,
  } from "../lib/api";

  type FetchState = "loading" | "error" | "ready";

  let plane = $state<ControlPlane | undefined>();
  let controlState = $state<FetchState>("loading");
  let items = $state<TaskSummary[]>([]);
  let listState = $state<FetchState>("loading");

  let busy = $state<"" | "merge" | "release">("");
  let result = $state<ActionResult>({ kind: "none", message: "" });
  let modalOpen = $state(false);
  let releaseError = $state("");

  let controlController: AbortController | undefined;
  let listController: AbortController | undefined;
  let controlLoaded = false;
  let listLoaded = false;

  function aborted(error: unknown): boolean {
    return error instanceof DOMException && error.name === "AbortError";
  }

  function reason(error: unknown): string {
    return error instanceof Error && error.message !== ""
      ? error.message
      : "原因不明のエラー";
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

  // No confirmation step: nothing is chosen, so nothing is asked. One request
  // per candidate, and a partial failure is reported as such.
  async function onmerge() {
    const targets = plane?.mergeable ?? [];
    if (targets.length === 0 || busy !== "") {
      return;
    }
    busy = "merge";
    result = { kind: "none", message: "" };
    const outcomes = await Promise.allSettled(
      targets.map((target) => postMerge(target.id)),
    );
    const issued = outcomes.filter((one) => one.status === "fulfilled").length;
    const failures = outcomes
      .filter((one) => one.status === "rejected")
      .map((one) => reason(one.reason));
    busy = "";
    result =
      failures.length === 0
        ? { kind: "success", message: `merge task を ${issued} 件発行しました` }
        : {
            kind: "error",
            message: `merge task を ${issued} 件発行し、${failures.length} 件失敗しました: ${[...new Set(failures)].join(" / ")}`,
          };
    await loadBoth();
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
    busy = "release";
    releaseError = "";
    try {
      const released = await postRelease(productId, tag);
      closeModal();
      result = {
        kind: "success",
        message: `${released.product_id} を ${released.tag} で release しました (${released.released.length} 件)`,
      };
      busy = "";
      await loadBoth();
    } catch (error) {
      // A refused release is corrected where the tag was typed.
      releaseError = reason(error);
      busy = "";
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
    {onmerge}
    {onrelease}
    onretry={() => void loadControl()}
  />
  <StatusTaskList
    fetchState={listState}
    {items}
    onretry={() => void loadList()}
  />
</div>

{#if modalOpen}
  <ReleaseModal
    releasable={plane?.releasable ?? []}
    busy={busy === "release"}
    error={releaseError}
    onclose={closeModal}
    onconfirm={(productId, tag) => void onconfirmRelease(productId, tag)}
  />
{/if}
