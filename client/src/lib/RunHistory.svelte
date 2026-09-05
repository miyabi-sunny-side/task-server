<script lang="ts">
  import { checkLabel, fetchRuns, type Run } from "./api";
  let { taskId }: { taskId: string } = $props();
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
      runs = [...runs, ...page.runs];
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
  data-field="runs"
  ontoggle={(event) => {
    if (event.currentTarget.open && !loaded) void load();
  }}
>
  <summary>実行履歴</summary>
  {#if busy}<p class="caption" role="status">読み込み中…</p>{/if}
  {#if error}<p class="state error">{error}</p>
    <button class="btn" type="button" onclick={() => void load()}>再試行</button
    >{/if}
  {#each runs as run (run.id)}
    <details class="run">
      <summary>{run.at} · {run.outcome ?? run.source}</summary>
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
