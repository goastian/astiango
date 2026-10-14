export const views = [
  { id: "overview", path: "/overview", group: "Sistema", label: "Vista general", icon: "overview", description: "Estado de los componentes conectados a esta instancia." },
  { id: "components", path: "/components", group: "Sistema", label: "Componentes", icon: "components", description: "Fuentes de estado disponibles y componentes pendientes de integración." },
  { id: "crawler-overview", path: "/crawler/overview", group: "Crawler", label: "Resumen del rastreo", icon: "globe", description: "Producción, cobertura e incidencias de la ejecución actual." },
  { id: "sites", path: "/crawler/sites", group: "Crawler", label: "Sitios y URLs", icon: "sites", description: "Busca sitios, consulta su estado e inspecciona sus solicitudes." },
  { id: "activity", path: "/crawler/activity", group: "Crawler", label: "Actividad", icon: "activity", description: "Eventos recientes, errores y seguimiento del sitio seleccionado." },
  { id: "policies", path: "/crawler/policies", group: "Crawler", label: "Cola y políticas", icon: "queue", description: "Elegibilidad, cortesía, robots, reintentos y revisión de cuarentenas." },
  { id: "performance", path: "/crawler/performance", group: "Crawler", label: "Rendimiento", icon: "chart", description: "Transporte, memoria, extracción, pipeline y publicación de métricas." },
  { id: "storage", path: "/crawler/storage", group: "Crawler", label: "Almacenamiento", icon: "storage", description: "Capturas WARC, segmentos durables, cuotas y publicación." },
  { id: "configuration", path: "/crawler/configuration", group: "Crawler", label: "Configuración", icon: "settings", description: "Parámetros activos e identidad de la ejecución. Consulta sin modificar el crawl." },
  { id: "indexing", path: "/indexing/overview", group: "Indexing", label: "Indexing overview", icon: "index", description: "Committed documents, input progress, resources and coexistence with the crawler." },
  { id: "indexing-shards", path: "/indexing/shards", group: "Indexing", label: "Input shards", icon: "storage", description: "Sealed segments, checkpoints and errors in the local ingestion queue." },
  { id: "indexing-generations", path: "/indexing/generations", group: "Indexing", label: "Generations", icon: "storage", description: "Validated generations, active selector and retained rollback." },
  { id: "indexing-configuration", path: "/indexing/configuration", group: "Indexing", label: "Indexing configuration", icon: "settings", description: "Worker budgets, pilot limits and publication scope." },
  { id: "webgraph", path: "/webgraph/overview", group: "Webgraph", label: "Webgraph overview", icon: "graph", description: "Captured inputs, stored links and graph activity." },
  { id: "webgraph-inputs", path: "/webgraph/inputs", group: "Webgraph", label: "Graph inputs", icon: "storage", description: "Checkpoints, input budgets and retained failures." },
  { id: "webgraph-centrality", path: "/webgraph/centrality", group: "Webgraph", label: "Centrality", icon: "chart", description: "Scores from local graph cohorts; consumer adoption is separate." },
  { id: "webgraph-generations", path: "/webgraph/generations", group: "Webgraph", label: "Generations", icon: "storage", description: "Validated generations, active selector and retained rollback." },
  { id: "webgraph-configuration", path: "/webgraph/configuration", group: "Webgraph", label: "Graph configuration", icon: "settings", description: "Graph worker, memory, disk and centrality budgets." },
  { id: "benchmarks", path: "/tools/benchmarks", group: "Herramientas", label: "Pruebas de capacidad", icon: "experiment", description: "Mediciones locales y comparaciones reproducibles, separadas de la operación." },
];

const legacyPaths: Record<string, string> = {
  resumen: "/crawler/overview", sitios: "/crawler/sites", actividad: "/crawler/activity",
  almacenamiento: "/crawler/storage", "banco-c01": "/tools/benchmarks", "config-panel": "/crawler/configuration",
};

export function resolveView(hash: string) {
  const path = hash.replace(/^#/, "");
  return views.find(view => view.path === (legacyPaths[path] ?? (path || "/overview")))?.id ?? "not-found";
}

export const componentCatalog = [
  { name: "Crawler", purpose: "Adquisición web, descubrimiento y captura durable.", source: "crawler" },
  { name: "Indexación", purpose: "Procesamiento de documentos y publicación del índice.", source: "indexer" },
  { name: "Grafo web", purpose: "Enlaces, centralidad y señales para planificación.", source: "webgraph" },
  { name: "Búsqueda y API", purpose: "Consultas, ranking y disponibilidad del buscador.", source: null },
  { name: "Coordinación", purpose: "Trabajos distribuidos y capacidad de los nodos.", source: null },
  { name: "Live index", purpose: "Actualización de contenido reciente y publicación.", source: null },
];

export const benchmarkNames: Record<string, string> = {
  C01: "Capacidad y recursos", C02: "Solicitudes y resultados", C03: "Publicación de métricas",
  C04: "Transporte HTTP y DNS", C05: "Admisión y memoria", C06: "Extracción de enlaces",
  C07: "Pipeline y contrapresión", C08: "Archivo y recuperación",
  "C09–C10": "Captura y frontier durable", "C11–C13": "Identidad, cortesía y robots",
  "C14–C20": "Revalidación y planificación",
};
