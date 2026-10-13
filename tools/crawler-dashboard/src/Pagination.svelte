<script lang="ts">
  let { page, pages, total, pageSize, onpage }: {
    page: number; pages: number; total: number; pageSize: number; onpage: (page: number) => void;
  } = $props();
</script>
<nav class="pagination" aria-label="Paginación">
  <span>{total ? (page - 1) * pageSize + 1 : 0}–{Math.min(page * pageSize, total)} de {total}</span>
  <div><button class="button" disabled={page <= 1} onclick={() => onpage(page - 1)}>Anterior</button><span>Página {page} de {pages}</span><button class="button" disabled={page >= pages} onclick={() => onpage(page + 1)}>Siguiente</button></div>
</nav>
