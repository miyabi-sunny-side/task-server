<script lang="ts">
  import type { PendingRelease } from "./api";

  // The outstanding releases: at most one per product, issued by the landing
  // and finished by the tag a worker cut. Drawn like the merge trains — one
  // card per release wearing its status badge, the level beside it, and the
  // reason under the title when it stopped — because a release that stopped
  // holds every later landing of its product back, exactly as a jammed merge
  // does (DESIGN.md, Releases).
  let {
    pending = [],
  }: {
    pending?: PendingRelease[];
  } = $props();
</script>

{#if pending.length > 0}
  <div class="readout" data-block="releases" data-readout="releases">
    <p class="head">
      <span class="caption">release</span>
      <span class="pill" data-count>{pending.length}</span>
    </p>
    <ul class="cards">
      {#each pending as item (item.id)}
        <li>
          <a
            class="card"
            href={`/tasks/${item.id}`}
            data-release={item.product_id}
          >
            <span class="line">
              <span class="name">{item.product_id}</span>
              <span class="tail">
                <span class="badge" data-level>{item.release_level}</span>
                <span class="badge">{item.status}</span>
              </span>
            </span>
            {#if item.status === "blocked" && (item.verification ?? "") !== ""}
              <span class="reason" data-reason>{item.verification}</span>
            {/if}
          </a>
        </li>
      {/each}
    </ul>
  </div>
{/if}

<style lang="sass">
  .readout
    display: flex
    flex-direction: column
    gap: var(--sp-2)

  .head
    display: flex
    align-items: center
    gap: var(--sp-2)
    margin: 0

  .caption
    margin: 0
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .card
    flex-direction: column
    align-items: stretch
    gap: var(--sp-2)

  .line
    display: flex
    align-items: baseline
    justify-content: space-between
    gap: var(--sp-2)

  .name
    overflow-wrap: anywhere

  .tail
    display: flex
    flex-shrink: 0
    align-items: baseline
    gap: var(--sp-2)

  // Neutral: a tag that could not be cut is an ordinary outcome of shipping
  // work, not a failure of this app.
  .reason
    font-size: var(--fs-sm)
    line-height: 1.5
    white-space: pre-line
    overflow-wrap: anywhere
</style>
