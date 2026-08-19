<script lang="ts">
  const footer = [
    ['/privacy-and-happy-lawyers', 'Terms & Privacy'],
    ['https://astiango.com/beta/api/docs/swagger', 'API'],
  ] as const;
</script>

<footer class="site-footer">
  <div class="footer-brand"><span>Astian</span><b>GO</b><small>Private search for the open web.</small></div>
  <nav aria-label="Footer links">
    {#each footer as [url, name]}
      <a href={url}>{name}</a>
    {/each}
    <a href="/webmasters">Webmasters</a>
  </nav>
  <span class="copyright">© 2026 Astian</span>
</footer>

<style>
  .site-footer { display: grid; grid-template-columns: minmax(0, 1fr) auto auto; align-items: center; gap: var(--space-6); margin-top: auto; padding: var(--space-6) max(var(--space-5), calc((100vw - 1240px) / 2)); border-top: var(--rule) solid var(--color-rule); background: var(--color-paper); color: var(--color-muted); font-family: var(--font-body); }
  .footer-brand { display: flex; align-items: baseline; gap: var(--space-2); color: var(--color-accent); font: 700 18px var(--font-display); letter-spacing: -.07em; } .footer-brand b { color: var(--color-coral); } small { color: var(--color-muted); font: 500 12px var(--font-body); letter-spacing: 0; } nav { display: flex; gap: var(--space-4); } nav a { color: var(--color-ink-soft); font: 600 12px var(--font-body); text-decoration: none; white-space: nowrap; } nav a:hover { color: var(--color-accent-strong); } .copyright { font: 500 12px var(--font-body); white-space: nowrap; } a:focus-visible { outline: 3px solid var(--color-focus); outline-offset: 3px; border-radius: var(--radius-sm); }
  @media (max-width: 720px) { .site-footer { grid-template-columns: 1fr; gap: var(--space-3); } .footer-brand { align-items: flex-start; flex-wrap: wrap; } nav { flex-wrap: wrap; } }
</style>
