<script lang="ts">
  import Icon, { ICON_NAMES } from "./Icon.svelte";
</script>

<ul class="catalog">
  {#each ICON_NAMES as name (name)}
    <li class="entry">
      <span class="specimen"><Icon {name} /></span>
      <span class="entry-name">{name}</span>
    </li>
  {/each}
</ul>

<style lang="sass">
  .catalog
    display: grid
    grid-template-columns: repeat(auto-fill, minmax(72px, 1fr))
    gap: var(--sp-2)
    margin: var(--sp-4) 0 0
    padding: 0
    list-style: none

  .entry
    display: flex
    flex-direction: column
    align-items: center
    gap: var(--sp-1)

  .specimen
    display: inline-flex
    align-items: center
    justify-content: center
    width: 36px
    height: 36px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface-raised)

  .entry-name
    font-size: var(--fs-xs)
    color: var(--c-muted)
</style>
