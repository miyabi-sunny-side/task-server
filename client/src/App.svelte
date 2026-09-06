<script lang="ts">
  import Header from "./lib/Header.svelte";
  import { initRouter, router } from "./lib/router.svelte";
  import Detail from "./pages/Detail.svelte";
  import Closed from "./pages/Closed.svelte";
  import Home from "./pages/Home.svelte";
  import Products from "./pages/Products.svelte";

  $effect(() => {
    initRouter();
  });
</script>

<svelte:head>
  <title>Task Server</title>
</svelte:head>

<Header />

<main>
  {#if router.index === 1}
    <Detail id={router.params.id} />
  {:else if router.index === 2}
    <Closed />
  {:else if router.index === 3}
    <Products />
  {:else}
    <Home />
  {/if}
</main>
