<script lang="ts">
  import { fetchProducts, type Product } from "../lib/api";
  import Icon from "../lib/Icon.svelte";
  import { startAutoReload } from "../lib/auto-reload";

  let items = $state<Product[]>([]);
  let query = $state("");
  let matches = $derived(
    items.filter((item) => item.id.toLowerCase().includes(query.toLowerCase())),
  );
  let fetchState = $state<"loading" | "error" | "ready">("loading");
  let controller: AbortController | undefined;
  let loaded = false;

  async function load() {
    controller?.abort();
    const request = new AbortController();
    controller = request;
    if (!loaded) fetchState = "loading";
    try {
      const products = await fetchProducts(request.signal);
      if (request.signal.aborted) return;
      items = products;
      fetchState = "ready";
      loaded = true;
    } catch {
      // Background failures preserve the current readout, like the task lists.
      if (!loaded && !request.signal.aborted) fetchState = "error";
    }
  }

  let listState = $derived(
    fetchState === "ready"
      ? items.length === 0
        ? "empty"
        : "success"
      : fetchState,
  );

  $effect(() => {
    void load();
    const stop = startAutoReload(() => void load());
    return () => {
      stop();
      controller?.abort();
    };
  });
</script>

<div class="content">
  <h1 id="products-title">プロダクト一覧</h1>
  <label for="product-search">プロダクト名で検索</label>
  <div class="search-field">
    <span class="search-icon"><Icon name="search" /></span>
    <input id="product-search" type="search" bind:value={query} />
  </div>
  <section
    aria-labelledby="products-title"
    data-region="products"
    data-state={listState}
  >
    {#if listState === "loading"}
      <p class="state" role="status">
        <span class="spinner" aria-hidden="true"></span>読み込み中…
      </p>
    {:else if listState === "empty"}
      <p class="state" role="status">登録済みのプロダクトがありません</p>
    {:else if listState === "error"}
      <div class="state-wrap">
        <p class="state error" role="alert">読み込みに失敗しました</p>
        <button class="btn" type="button" onclick={() => void load()}
          >再試行</button
        >
      </div>
    {:else if matches.length === 0}
      <p class="state" role="status">一致するプロダクトがありません</p>
    {:else}
      <ul class="cards">
        {#each matches as item (item.id)}
          <li>
            <details class="card">
              <summary>{item.id}</summary>
              <dl>
                <div>
                  <dt>リポジトリ</dt>
                  <dd>{item.repository}</dd>
                </div>
                <div>
                  <dt>説明</dt>
                  <dd class="description">{item.description || "説明なし"}</dd>
                </div>
                <div>
                  <dt>ローカルパス</dt>
                  <dd>{item.local_path ?? "未設定"}</dd>
                </div>
                <div>
                  <dt>リリース</dt>
                  <dd>
                    {item.releases === true
                      ? "公開"
                      : item.releases === false
                        ? "公開しない"
                        : "未設定"}
                  </dd>
                </div>
                <div>
                  <dt>アーカイブ</dt>
                  <dd>{item.archived ? "アーカイブ済み" : "有効"}</dd>
                </div>
                <div>
                  <dt>アーカイブ日時</dt>
                  <dd>{item.archived_at ?? "未設定"}</dd>
                </div>
              </dl>
            </details>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

<style lang="sass">
  h1, summary
    margin: 0
    font-size: var(--fs-md)
    font-weight: 500
    line-height: 1.2
    overflow-wrap: anywhere

  h1
    margin-bottom: var(--sp-3)

  label
    display: block
    margin-bottom: var(--sp-2)
    color: var(--c-muted)
    font-size: var(--fs-xs)
    line-height: 1.4

  .search-field
    position: relative
    margin-bottom: var(--sp-3)

  .search-icon
    position: absolute
    left: var(--sp-2)
    top: 50%
    transform: translateY(-50%)
    color: var(--c-muted)
    pointer-events: none

  input
    width: 100%
    min-width: 0
    padding: var(--sp-2)
    padding-left: calc(var(--sp-2) * 2 + 1.2em)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface)
    color: var(--c-on-surface)
    font-size: var(--fs-lg)
    line-height: 1.6

    &:focus
      border-color: var(--c-accent)

  .card
    display: block
    min-width: 0

    &:hover
      background: var(--c-surface-raised)

  summary
    cursor: pointer
    min-height: 36px
    align-content: center

  dl
    display: flex
    flex-direction: column
    gap: var(--sp-2)
    margin: var(--sp-2) 0 0

  dt
    color: var(--c-muted)
    font-size: var(--fs-xs)
    line-height: 1.4

  dd
    margin: 0
    font-size: var(--fs-sm)
    line-height: 1.5
    overflow-wrap: anywhere

  .description
    white-space: pre-line
</style>
