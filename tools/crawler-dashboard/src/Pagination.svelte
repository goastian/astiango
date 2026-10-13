<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale } = getI18n();

  let { page, pages, total, pageSize, onpage }: {
    page: number; pages: number; total: number; pageSize: number; onpage: (page: number) => void;
  } = $props();
</script>
<nav class="pagination" aria-label={$t("Paginación")}>
  <span>{$t("{p0}–{p1} de {p2}", { p0: total ? (page - 1) * pageSize + 1 : 0, p1: Math.min(page * pageSize, total), p2: total })}</span>
  <div><button class="button" disabled={page <= 1} onclick={() => onpage(page - 1)}>{$t("Anterior")}</button><span>{$t("Página {p0} de {p1}", { p0: page, p1: pages })}</span><button class="button" disabled={page >= pages} onclick={() => onpage(page + 1)}>{$t("Siguiente")}</button></div>
</nav>
