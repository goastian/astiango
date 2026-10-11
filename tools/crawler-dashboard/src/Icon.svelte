<script lang="ts">
  let { name, size = 19 }: { name: string; size?: number } = $props();
  const paths: Record<string, string> = {
    refresh:
      "M20 7v5h-5M4 17v-5h5M6.1 7a7 7 0 0 1 11.6-1L20 9M4 15l2.3 3A7 7 0 0 0 18 17",
    pause: "M8 5v14M16 5v14",
    play: "m8 5 11 7-11 7Z",
    download: "M12 3v12m-5-5 5 5 5-5M4 15v5h16v-5",
    settings: "M4 7h16M4 17h16M8 4v6M16 14v6",
    calendar: "M4 5h16v15H4ZM4 10h16M8 3v4M16 3v4",
    globe:
      "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0ZM3 12h18M12 3c5 5 5 13 0 18-5-5-5-13 0-18Z",
    close: "m6 6 12 12M6 18 18 6",
    clock: "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0ZM12 7v5l3 2",
    chevron: "m8 10 4 4 4-4",
  };
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"><path d={paths[name] ?? paths.globe} /></svg
>
