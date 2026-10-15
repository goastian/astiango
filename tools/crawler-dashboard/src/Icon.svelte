<script lang="ts">
  let { name, size = 19 }: { name: string; size?: number } = $props();
  const paths: Record<string, string> = {
    refresh:
      "M20 7v5h-5M4 17v-5h5M6.1 7a7 7 0 0 1 11.6-1L20 9M4 15l2.3 3A7 7 0 0 0 18 17",
    pause: "M8 5v14M16 5v14",
    play: "m8 5 11 7-11 7Z",
    download: "M12 3v12m-5-5 5 5 5-5M4 15v5h16v-5",
    settings: "M4 7h16M4 17h16M8 4v6M16 14v6",
    search: "M21 21l-4.3-4.3M19 11a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z",
    calendar: "M4 5h16v15H4ZM4 10h16M8 3v4M16 3v4",
    globe:
      "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0ZM3 12h18M12 3c5 5 5 13 0 18-5-5-5-13 0-18Z",
    close: "m6 6 12 12M6 18 18 6",
    clock: "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0ZM12 7v5l3 2",
    chevron: "m8 10 4 4 4-4",
    overview: "M3 3h7v7H3ZM14 3h7v7h-7ZM3 14h7v7H3ZM14 14h7v7h-7Z",
    components: "M4 4h6v6H4ZM14 4h6v6h-6ZM4 14h6v6H4ZM14 14h6v6h-6Z",
    sites: "M3 7h18v14H3ZM7 3h10v4M7 11h3M14 11h3M7 16h3M14 16h3",
    activity: "M3 12h4l3-7 4 14 3-7h4",
    index: "M4 3h13l3 3v15H4ZM8 8h8M8 12h8M8 16h5",
    queue: "M8 5h13M8 12h13M8 19h13M3 5h1M3 12h1M3 19h1",
    chart: "M4 3v17h17M8 15V9M13 15V5M18 15v-4",
    graph: "M5 5h4v4H5ZM15 4h4v4h-4ZM10 15h4v4h-4ZM9 7l6-1M7 9l4 6M17 8l-4 7",
    storage: "M3 5h18v6H3ZM3 14h18v6H3ZM17 8h1M17 17h1",
    experiment: "M9 3h6M10 3v7L4 19c-.7 1 .1 2 1 2h14c.9 0 1.7-1 1-2l-6-9V3M7 16h10",
    menu: "M4 6h16M4 12h16M4 18h16",
    arrow: "M4 12h16m-6-6 6 6-6 6",
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
