import { createServer } from "node:http";
import { execFile } from "node:child_process";
import { readFile, stat, mkdir, open, rename } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { dirname, extname, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { isBenchmarkReport } from "./benchmark-report.mjs";
import { snapshotCache, paginate, aggregateMetrics } from "./observability.mjs";
import { resourceSeries } from "./series.mjs";

const appDirectory = dirname(fileURLToPath(import.meta.url));
const contentTypes = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2",
};
function respondJson(response, status, value) {
  response.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Cache-Control": "no-store",
  });
  response.end(JSON.stringify(value));
}
function processAlive(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return null;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === "EPERM";
  }
}

export function createDashboardServer({
  statePath = process.env.CRAWLER_STATE_PATH ||
    resolve(appDirectory, "../../data/crawler-local/status.json"),
  benchmarkPath = process.env.CRAWLER_BENCHMARK_PATH || null,
  distDirectory = resolve(appDirectory, "dist"),
  seriesPath = process.env.CRAWLER_SERIES_PATH || null,
  operationsPath = process.env.CRAWLER_OPERATIONS_PATH || resolve(appDirectory, "../../data/crawler-operations/report.json"),
  soakPath = process.env.CRAWLER_SOAK_PATH || resolve(appDirectory, "../../data/crawler-operations/soak.json"),
  indexingPath = process.env.INDEXER_STATE_PATH || resolve(appDirectory, "../../data/indexer-shared/status.json"),
  webgraphPath = process.env.WEBGRAPH_STATE_PATH || resolve(appDirectory, "../../data/webgraph-local/status.json"),
  webgraphLedgerPath = process.env.WEBGRAPH_LEDGER_STATE_PATH || resolve(appDirectory, "../../data/webgraph-ingestion/status.json"),
} = {}) {
  let previewBusy = false;
  const searchApi = "http://127.0.0.1:3210";
  const searchControlToken = resolve(appDirectory, "../../data/search-api/control-token");
  let searchReadCache = null;
  let searchReadPromise = null;
  async function searchObservation() {
    if (searchReadCache && Date.now()-searchReadCache.at<500) return searchReadCache.value;
    if (searchReadPromise) return searchReadPromise;
    searchReadPromise = (async()=>{
      const upstream=await fetch(searchApi+"/status",{signal:AbortSignal.timeout(3000)});
      if(!upstream.ok || Number(upstream.headers.get("content-length")??0)>512*1024) throw Error("Search observation unavailable");
      const data=await upstream.arrayBuffer();
      if(data.byteLength>512*1024) throw Error("Search observation exceeds budget");
      const value=JSON.parse(Buffer.from(data).toString("utf8"));
      searchReadCache={at:Date.now(),value};
      return value;
    })();
    try { return await searchReadPromise; } finally { searchReadPromise=null; }
  }
  const distRoot = resolve(distDirectory);
  const observations = snapshotCache(statePath);
  const series = resourceSeries(seriesPath);
  const generations = snapshotCache(resolve(dirname(indexingPath), "index/generations/status.json"), 256 * 1024, value => value.version === 1 && Array.isArray(value.generations) && value.generations.length <= 32);
  const indexing = snapshotCache(indexingPath, 256 * 1024, snapshot => snapshot.schema_version === 1 && snapshot.component === "indexer" && Array.isArray(snapshot.shards) && snapshot.shards.length <= 128 && snapshot.totals && snapshot.configuration && snapshot.source && snapshot.publication);
  const webgraph = snapshotCache(webgraphPath, 256 * 1024, snapshot => snapshot.schema_version === 1 && snapshot.component === "webgraph" && Array.isArray(snapshot.inputs) && snapshot.inputs.length <= 64 && snapshot.totals && snapshot.configuration);
  const graphLedger = snapshotCache(webgraphLedgerPath, 256 * 1024, value => {
    const snapshot = value.snapshot ?? value;
    return snapshot.schema_version === 1 && snapshot.component === "webgraph" && snapshot.scope === "current_version_graph" && Array.isArray(snapshot.inputs) && snapshot.inputs.length <= 64 && snapshot.totals && snapshot.configuration;
  });
  return createServer(async (request, response) => {
    response.setHeader("X-Content-Type-Options", "nosniff");
    response.setHeader("Referrer-Policy", "no-referrer");
    response.setHeader(
      "Content-Security-Policy",
      "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
    );
    if (request.method !== "GET" && request.method !== "HEAD" && !(request.method === "POST" && ["/api/quarantine/release", "/api/search/control"].includes(request.url))) {
      response.setHeader("Allow", "GET, HEAD");
      respondJson(response, 405, { error: "Método no permitido." });
      return;
    }
    let pathname;
    try {
      pathname = decodeURIComponent((request.url || "/").split("?")[0]);
    } catch {
      respondJson(response, 400, { error: "Ruta no válida." });
      return;
    }
    if (
      pathname.includes("\0") ||
      pathname.includes("\\") ||
      pathname.split("/").includes("..")
    ) {
      respondJson(response, 403, { error: "Ruta no permitida." });
      return;
    }
    if (pathname === "/healthz" || pathname === "/api/health") {
      respondJson(response, 200, {
        status: "ok",
        service: "crawler-dashboard",
      });
      return;
    }
    if (pathname === "/api/operations") {
      try {
        if ((await stat(operationsPath)).size > 1024 * 1024) throw new Error("Operations report exceeds size budget");
        const report = JSON.parse(await readFile(operationsPath, "utf8"));
        if (report.version !== 1 || report.kind !== "isolated_local_verification") throw new Error("Unsupported operations report");
        let soak = null;
        try {
          if ((await stat(soakPath)).size > 65536) throw new Error("Soak report exceeds size budget");
          soak = JSON.parse(await readFile(soakPath, "utf8"));
          if (soak.version !== 1) throw new Error("Unsupported soak report");
        } catch (error) { if (error.code !== "ENOENT") throw error; }
        respondJson(response, 200, { report, soak });
      } catch (error) {
        respondJson(response, error.code === "ENOENT" ? 404 : 503, { code: "OPERATIONS_UNAVAILABLE", error: "Operational verification is unavailable." });
      }
      return;
    }
    if (pathname.startsWith("/api/search/")) {
      const routes = {"/api/search/status":"/status", "/api/search/metrics":"/metrics", "/api/search/nodes":"/nodes", "/api/search/generations":"/generations", "/api/search/query":"/v1/web/search", "/api/search/control":"/control"};
      const route = routes[pathname];
      if (!route) { respondJson(response,404,{code:"SEARCH_ROUTE_UNKNOWN"}); return; }
      try {
        if (["/api/search/status","/api/search/metrics","/api/search/nodes","/api/search/generations"].includes(pathname)) {
          respondJson(response,200,await searchObservation());return;
        }
        const headers = {};
        let body;
        if (pathname === "/api/search/control") {
          if (request.method !== "POST" || !["127.0.0.1","::1","::ffff:127.0.0.1"].includes(request.socket.remoteAddress)
            || request.headers.origin !== `http://${request.headers.host}` || !request.headers["content-type"]?.startsWith("application/json")) {
            respondJson(response,403,{error:"Local same-origin control required"}); return;
          }
          const chunks=[];let size=0;
          for await (const chunk of request) { size+=chunk.length; if(size>8192) {respondJson(response,413,{error:"Control request exceeds budget"});return;} chunks.push(chunk); }
          body=Buffer.concat(chunks);headers["Content-Type"]="application/json";
        }
        if (pathname === "/api/search/query" || body) {
          headers.Authorization=`Bearer ${(await readFile(searchControlToken,"utf8")).trim()}`;
          headers["X-Search-Traffic"]="probe";
        }
        const query = pathname === "/api/search/query" ? new URL(request.url,"http://localhost").search : "";
        const upstream = await fetch(searchApi+route+query,{method:body?"POST":"GET",headers,body,signal:AbortSignal.timeout(4000)});
        if(Number(upstream.headers.get("content-length")??0)>512*1024) throw Error("Search response exceeds budget");
        const data=await upstream.arrayBuffer();
        if(data.byteLength>512*1024) throw Error("Search response exceeds budget");
        if(upstream.headers.has("retry-after")) response.setHeader("Retry-After",upstream.headers.get("retry-after"));
        respondJson(response,upstream.status,JSON.parse(Buffer.from(data).toString("utf8")));
      } catch { respondJson(response,503,{code:"SEARCH_UNAVAILABLE",error:"Search/API is unavailable or its deadline expired."}); }
      return;
    }
    if (pathname === "/api/webgraph" || pathname === "/api/webgraph/inputs") {
      try {
        let observation;
        let reader = graphLedger;
        try { observation = await graphLedger.get(); }
        catch (error) {
          if (error.code !== "ENOENT") throw error;
          reader = webgraph;
          observation = await webgraph.get();
        }
        const snapshot = observation.snapshot.snapshot ?? observation.snapshot;
        let alive = processAlive(snapshot.pid);
        if (alive && snapshot.start_ticks) {
          try {
            const proc = await readFile(`/proc/${snapshot.pid}/stat`, "utf8");
            alive = proc.slice(proc.lastIndexOf(")") + 2).split(" ")[19] === snapshot.start_ticks;
          } catch { alive = false; }
        }
        if (pathname.endsWith("/inputs")) {
          try {
            const parameters = new URL(request.url, "http://localhost").searchParams;
            respondJson(response, 200, { run_id: snapshot.run_id, ...paginate(snapshot.inputs.map(row => ({ ...row, status: row.state })), parameters, ["id", "file", "error"]) });
          } catch (error) { respondJson(response, 400, { error: error.message }); }
        } else {
          respondJson(response, 200, { snapshot, meta: { read_at: new Date().toISOString(), modified_at: observation.modifiedAt, process_alive: alive, cache: { ...reader.counters } } });
        }
      } catch (error) {
        respondJson(response, 503, { code: "WEBGRAPH_UNAVAILABLE", error: "Webgraph state is unavailable." });
      }
      return;
    }
    if (pathname === "/api/indexing/generations") {
      try {
        const value = await generations.get();
        respondJson(response, 200, value.snapshot);
      } catch (error) { respondJson(response, error.code === "ENOENT" ? 404 : 503, { error: "Generation status unavailable" }); }
      return;
    }
    if (pathname === "/api/indexing/preview") {
      const query = new URL(request.url, "http://localhost").searchParams.get("q") || "";
      if (!query.trim() || Buffer.byteLength(query,"utf8") > 4096) { respondJson(response, 400, { error: "Invalid preview query." }); return; }
      if (previewBusy) { respondJson(response, 429, { error: "Preview busy; retry shortly." }); return; }
      let generationActive = false;
      try { generationActive = !!(await generations.get()).snapshot.active; } catch {}
      previewBusy = true;
      const previewSelector = resolve(dirname(indexingPath), "preview-reader");
      const previewBinary = process.env.INDEXER_BINARY || await stat(previewSelector).then(() => previewSelector).catch(() => resolve(appDirectory, "../../target/release/astiango-indexer"));
      execFile(previewBinary, [resolve(dirname(indexingPath), "current-job.toml"), generationActive ? "--generation-query" : "--query", query], { timeout: 5000, maxBuffer: 256 * 1024, env: { ...process.env, RUST_LOG: "error" } }, (error, stdout) => {
        previewBusy = false;
        if (error) { respondJson(response, 503, { error: "Local preview unavailable or ingestion busy; retry shortly." }); return; }
        try { respondJson(response, 200, JSON.parse(stdout)); } catch { respondJson(response, 503, { error: "Invalid native preview response." }); }
      });
      return;
    }
    if (pathname === "/api/indexing" || pathname === "/api/indexing/shards") {
      try {
        const observation = await indexing.get();
        const { snapshot } = observation;
        let alive = processAlive(snapshot.pid);
        if (alive && snapshot.start_ticks) {
          try {
            const proc = await readFile(`/proc/${snapshot.pid}/stat`, "utf8");
            alive = proc.slice(proc.lastIndexOf(")") + 2).split(" ")[19] === snapshot.start_ticks;
          } catch { alive = false; }
        }
        if (pathname.endsWith("/shards")) {
          const parameters = new URL(request.url, "http://localhost").searchParams;
          try {
            respondJson(response, 200, { run_id: snapshot.run_id, ...paginate(snapshot.shards.map(row => ({ ...row, status: row.state })), parameters, ["id", "file", "error"]) });
          } catch (error) { respondJson(response, 400, { error: error.message }); }
        } else {
          respondJson(response, 200, { snapshot, meta: { read_at: new Date().toISOString(), modified_at: observation.modifiedAt, process_alive: alive, cache: { ...indexing.counters } } });
        }
      } catch (error) {
        respondJson(response, 503, { code: error.code === "ENOENT" ? "INDEXING_UNAVAILABLE" : "INDEXING_UNREADABLE", error: "Indexing state is unavailable." });
      }
      return;
    }
    if (pathname === "/api/quarantine/release") {
      if (request.method !== "POST") { respondJson(response, 405, { error: "Usa POST para solicitar una revisión." }); return; }
      const address = request.socket.remoteAddress;
      if (!["127.0.0.1", "::1", "::ffff:127.0.0.1"].includes(address) ||
          (request.headers.origin && request.headers.origin !== `http://${request.headers.host}`)) {
        respondJson(response, 403, { error: "Control disponible desde el dashboard local." }); return;
      }
      try {
        let body = "";
        for await (const chunk of request) {
          body += chunk.toString("utf8");
          if (Buffer.byteLength(body) > 16384) { respondJson(response, 413, { error: "Solicitud demasiado grande." }); return; }
        }
        const value = JSON.parse(body);
        const url = new URL(value.url);
        if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.href.length > 8192) throw new Error("URL inválida");
        url.hash = "";
        const id = randomUUID();
        const directory = resolve(dirname(statePath), "control");
        await mkdir(directory, { recursive: true });
        const temporary = resolve(directory, `${id}.tmp`);
        const file = await open(temporary, "wx", 0o600);
        try { await file.writeFile(JSON.stringify({ id, action: "release_quarantine", url: url.href })); await file.sync(); }
        finally { await file.close(); }
        await rename(temporary, resolve(directory, `${id}.request`));
        const folder = await open(directory, "r");
        try { await folder.sync(); } finally { await folder.close(); }
        respondJson(response, 202, { id, state: "queued", message: "Revisión en cola; el crawler la aplica sin reiniciar." });
      } catch { respondJson(response, 400, { error: "No se pudo guardar la solicitud de revisión." }); }
      return;
    }
    if (pathname.startsWith("/api/control/")) {
      const id = pathname.slice("/api/control/".length);
      if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id)) {
        respondJson(response, 400, { error: "Identificador de revisión inválido." }); return;
      }
      const directory = resolve(dirname(statePath), "control");
      try {
        const result = JSON.parse(await readFile(resolve(directory, `${id}.result`), "utf8"));
        respondJson(response, 200, result);
      } catch (error) {
        if (error.code !== "ENOENT") { respondJson(response, 503, { error: "Resultado de revisión no disponible." }); return; }
        try { await stat(resolve(directory, `${id}.request`)); respondJson(response, 200, { id, state: "queued" }); }
        catch { respondJson(response, 404, { error: "Revisión desconocida." }); }
      }
      return;
    }
    if (["/api/status", "/api/sites", "/api/events", "/api/series", "/metrics"].includes(pathname)) {
      try {
        const observation = await observations.get();
        const { snapshot } = observation;
        await series.observe(snapshot);
        if (pathname === "/api/series") {
          const run = new URL(request.url, "http://localhost").searchParams.get("run_id");
          const data = series.read();
          respondJson(response, 200, { ...data, samples: run ? data.samples.filter(sample => sample.run_id === run) : data.samples });
          return;
        }
        if (pathname === "/metrics") {
          response.writeHead(200, { "Content-Type": "text/plain; version=0.0.4; charset=utf-8", "Cache-Control": "no-store" });
          response.end(aggregateMetrics(snapshot, observations.counters));
          return;
        }
        if (pathname !== "/api/status") {
          try {
            const parameters = new URL(request.url, "http://localhost").searchParams;
            const rows = pathname === "/api/sites" ? snapshot.sites : snapshot.recent_events || [];
            const fields = pathname === "/api/sites" ? ["host", "current_url"] : ["kind", "message", "host", "url"];
            respondJson(response, 200, { run_id: snapshot.run_id, sampled_at: snapshot.updated_at, ...paginate(rows, parameters, fields) });
          } catch (error) { respondJson(response, 400, { error: error.message }); }
          return;
        }
        const meta = {
            read_at: new Date().toISOString(),
            modified_at: observation.modifiedAt,
            process_alive: processAlive(snapshot.pid),
            cache: { ...observations.counters },
          };
        response.writeHead(200, {
          "Content-Type": "application/json; charset=utf-8",
          "Cache-Control": "no-store",
        });
        response.end(`{"snapshot":${observation.source},"meta":${JSON.stringify(meta)}}`);
      } catch (error) {
        const missing = error.code === "ENOENT";
        respondJson(response, 503, {
          error: error.code === "STATE_SCHEMA" ? "El estado del crawler tiene un formato no compatible." : missing
            ? "El crawler todavía no ha publicado su estado."
            : "No se pudo leer el estado del crawler.",
          code: missing ? "STATE_UNAVAILABLE" : "STATE_UNREADABLE",
        });
      }
      return;
    }
    if (pathname === "/api/benchmarks") {
      if (!benchmarkPath) {
        respondJson(response, 404, {
          error: "El banco local no está habilitado en este dashboard.",
          code: "BENCHMARK_DISABLED",
        });
        return;
      }
      try {
        const [source, info] = await Promise.all([
          readFile(benchmarkPath, "utf8"),
          stat(benchmarkPath),
        ]);
        const report = JSON.parse(source);
        if (!isBenchmarkReport(report)) {
          respondJson(response, 503, {
            error: "El informe del banco tiene un formato no compatible.",
            code: "BENCHMARK_INVALID",
          });
          return;
        }
        respondJson(response, 200, {
          report,
          meta: {
            read_at: new Date().toISOString(),
            modified_at: info.mtime.toISOString(),
          },
        });
      } catch (error) {
        const missing = error.code === "ENOENT";
        const invalid = error instanceof SyntaxError;
        respondJson(response, 503, {
          error: missing
            ? "El banco todavía no ha publicado su informe."
            : "No se pudo leer el informe del banco.",
          code: missing
            ? "BENCHMARK_UNAVAILABLE"
            : invalid
              ? "BENCHMARK_INVALID"
              : "BENCHMARK_UNREADABLE",
        });
      }
      return;
    }
    if (pathname.startsWith("/api/")) {
      respondJson(response, 404, { error: "Ruta no encontrada." });
      return;
    }
    const filePath = resolve(
      distRoot,
      "." + (pathname === "/" ? "/index.html" : pathname),
    );
    if (!filePath.startsWith(distRoot + sep)) {
      respondJson(response, 403, { error: "Ruta no permitida." });
      return;
    }
    try {
      const body = await readFile(filePath);
      const etag = `"${createHash("sha256").update(body).digest("base64url")}"`;
      response.setHeader("ETag", etag);
      response.setHeader(
        "Cache-Control",
        pathname.startsWith("/assets/")
          ? "public, max-age=31536000, immutable"
          : "no-cache",
      );
      if (request.headers["if-none-match"] === etag) {
        response.writeHead(304);
        response.end();
        return;
      }
      response.writeHead(200, {
        "Content-Type":
          contentTypes[extname(filePath)] || "application/octet-stream",
        "Content-Length": body.length,
      });
      response.end(request.method === "HEAD" ? undefined : body);
    } catch (error) {
      respondJson(
        response,
        error.code === "ENOENT" || error.code === "EISDIR" ? 404 : 500,
        { error: "Archivo no disponible." },
      );
    }
  });
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  const port = Number(process.env.PORT || 3101);
  const host = process.env.HOST || "127.0.0.1";
  const server = createDashboardServer();
  server.listen(port, host, () =>
    console.log(`Crawler dashboard: http://${host}:${port}`),
  );
  for (const signal of ["SIGINT", "SIGTERM"])
    process.on(signal, () => server.close(() => process.exit(0)));
}
