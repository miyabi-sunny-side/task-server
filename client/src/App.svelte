<script lang="ts">
  import Header from "./lib/Header.svelte";
  import { loadSession } from "./lib/api";
  import { initRouter, router } from "./lib/router.svelte";
  import Detail from "./pages/Detail.svelte";
  import Home from "./pages/Home.svelte";

  $effect(() => {
    void loadSession();
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
  {:else}
    <Home />
  {/if}
</main>
