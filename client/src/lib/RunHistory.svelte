<script lang="ts">
  import { tick } from "svelte";
  import { checkLabel, fetchRun, fetchRuns, type Run } from "./api";
  let { taskId, selectedReport }: { taskId: string; selectedReport?: number } =
    $props();
  let opened = $state(false);
  let selectedError = $state(false);

  async function select(id: number) {
    opened = true;
    busy = true;
    error = "";
    selectedError = false;
    try {
      const run = await fetchRun(id);
      runs = [...runs.filter((r) => r.id !== id), run];
      if (!loaded) next = 0;
      loaded = true;
      await tick();
      document.getElementById(`run-${id}`)?.focus();
    } catch {
      error = "実行履歴の読み込みに失敗しました";
      selectedError = true;
    } finally {
      busy = false;
    }
  }
  $effect(() => {
    if (selectedReport !== undefined) void select(selectedReport);
  });
  let runs = $state<Run[]>([]);
  let next = $state<number | null>(null);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state("");

  async function load() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      const page = await fetchRuns(taskId, next ?? 0);
      runs = [
        ...new Map(
          [...runs, ...page.runs].map((run) => [run.id, run]),
        ).values(),
      ];
      next = page.next;
      loaded = true;
    } catch {
      error = "実行履歴の読み込みに失敗しました";
    } finally {
      busy = false;
    }
  }
</script>

<details
  class="history"
  bind:open={opened}
  data-field="runs"
  ontoggle={(event) => {
    if (event.currentTarget.open && !loaded && selectedReport === undefined)
      void load();
  }}
>
  <summary>実行履歴</summary>
  {#if busy}<p class="caption" role="status">読み込み中…</p>{/if}
  {#if error}<p class="state error">{error}</p>
    <button
      class="btn"
      type="button"
      onclick={() =>
        selectedError && selectedReport !== undefined
          ? void select(selectedReport)
          : void load()}>再試行</button
    >{/if}
  {#each runs as run (run.id)}
    <details class="run" open={run.id === selectedReport}>
      <summary id={`run-${run.id}`} tabindex="-1"
        >報告 #{run.id} · {run.at} · {run.outcome ?? run.source}</summary
      >
      {#if run.body}<pre data-field="report-original">{run.body}</pre>{/if}
      {#if run.claim_id}<p class="caption">実行 {run.claim_id}</p>{/if}
      {#if run.note}<p>{run.note}</p>{/if}
      <p class="caption">
        {run.worker ?? ""}
        {run.model ?? ""}
        {run.agent_secs != null ? `${run.agent_secs}s` : ""}
      </p>
      {#if run.commit_sha}<p class="caption">{run.commit_sha}</p>{/if}
      {#each run.checks ?? [] as check}<p>
          {checkLabel(check)}
        </p>{/each}
      {#if run.stdout_tail}<pre>{run.stdout_tail}</pre>{/if}
      {#if run.stderr_tail}<pre>{run.stderr_tail}</pre>{/if}
    </details>
  {:else}{#if loaded}<p class="caption">実行履歴はありません</p>{/if}{/each}
  {#if next !== null}<button
      class="btn"
      type="button"
      disabled={busy}
      onclick={() => void load()}>続きを読み込む</button
    >{/if}
</details>

<style lang="sass">
  .history
    margin-top: var(--sp-4)

  summary, .caption
    color: var(--c-muted)
    font-size: var(--fs-xs)
    line-height: 1.4

  summary
    cursor: pointer

  .run
    margin-top: var(--sp-3)

  p, pre
    margin: var(--sp-2) 0
    font-size: var(--fs-sm)
    line-height: 1.5

  pre
    white-space: pre-wrap

  .history
    overflow-wrap: anywhere
</style>
