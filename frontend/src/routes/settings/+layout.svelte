<div class="settings-page">
  <div class="settings-content">
    <slot />
  </div>
</div>

<style>
  .settings-page { display: flex; width: 100%; justify-content: center; padding: var(--space-12) var(--space-5); }
  .settings-content { width: min(100%, 760px); }
</style>
