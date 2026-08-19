<script lang="ts">
  import OpticSelector from '$lib/components/OpticSelector.svelte';
  import Searchbar from '$lib/components/Searchbar.svelte';
</script>

<section class="search-home" aria-labelledby="home-title">
  <div class="home-orbit" aria-hidden="true"></div>
  <div class="home-content">
    <h1 id="home-title" class="home-wordmark"><span>Astian</span><b>GO</b></h1>
    <p class="home-lede">Search the open web on your terms.</p>
    <div class="home-search">
    <Searchbar autofocus={true} />
    </div>
    <div class="private-notes" aria-label="Privacy features">
      <span><i>✓</i> No behavioural tracking</span><span><i>✓</i> Independent index</span><span><i>✓</i> Open source</span>
    </div>
  </div>
</section>

<style>
  .search-home { position: relative; display: grid; min-height: calc(100vh - 68px); overflow: clip; place-items: center; padding: var(--space-12) var(--space-5) var(--space-20); background: var(--color-paper); }
  .home-orbit { position: absolute; width: min(90vw, 980px); aspect-ratio: 1; border: var(--rule) solid var(--color-rule-soft); border-radius: 50%; transform: translateY(22%); } .home-orbit::before, .home-orbit::after { position: absolute; border: var(--rule) solid var(--color-rule-soft); border-radius: 50%; content: ''; inset: 11%; } .home-orbit::after { inset: 25%; border-color: var(--color-accent-soft); }
  .home-content { position: relative; display: flex; width: min(100%, 760px); flex-direction: column; align-items: center; } .home-wordmark { margin: 0; color: var(--color-accent); font: 700 clamp(4rem, 11vw, 7.8rem)/.86 var(--font-display); letter-spacing: -.09em; overflow-wrap: anywhere; } .home-wordmark b { color: var(--color-coral); font: inherit; } .home-lede { margin: var(--space-4) 0 var(--space-8); color: var(--color-ink-soft); font: 500 clamp(16px, 2vw, 19px)/1.4 var(--font-body); text-align: center; }
  .home-search { width: min(100%, 680px); } .private-notes { display: flex; flex-wrap: wrap; justify-content: center; gap: var(--space-2) var(--space-4); margin-top: var(--space-5); color: var(--color-muted); font: 600 12px var(--font-body); } .private-notes i { color: var(--color-accent-strong); font-style: normal; } .home-tools { align-self: flex-start; display: flex; align-items: center; gap: var(--space-2); margin-top: var(--space-10); color: var(--color-muted); font: 500 13px var(--font-body); }
  @media (max-width: 640px) { .search-home { align-items: start; padding-top: 20vh; } .home-orbit { width: 150vw; transform: translateY(4%); } .home-tools { align-self: center; align-items: center; flex-direction: column; text-align: center; } }
</style>
