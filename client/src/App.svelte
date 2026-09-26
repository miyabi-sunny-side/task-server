<script lang="ts">
  import Header from "./lib/Header.svelte";
  import { initRouter, router } from "./lib/router.svelte";
  import Detail from "./pages/Detail.svelte";
  import Closed from "./pages/Closed.svelte";
  import Home from "./pages/Home.svelte";
  import Products from "./pages/Products.svelte";
  import Ideas from "./pages/Ideas.svelte";
  import IdeaDetail from "./pages/IdeaDetail.svelte";

  let creating = $state(false);

  $effect(() => {
    initRouter();
  });
</script>

<svelte:head>
  <title>Task Server</title>
</svelte:head>

<Header oncreate={router.index === 0 ? () => (creating = true) : undefined} />

<main>
  {#if router.index === 1}
    <Detail id={router.params.id} />
  {:else if router.index === 2}
    <Closed />
  {:else if router.index === 3}
    <Products />
  {:else if router.index === 4 || router.index === 5}
    {#key router.index}<Ideas archived={router.index === 5} />{/key}
  {:else if router.index === 6}
    <IdeaDetail id={router.params.id} />
  {:else}
    <Home {creating} onclose={() => (creating = false)} />
  {/if}
</main>
