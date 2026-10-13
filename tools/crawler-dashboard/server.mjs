import { createServer } from "node:http";
import { readFile, stat, mkdir, open, rename } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { dirname, extname, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { isBenchmarkReport } from "./benchmark-report.mjs";

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
} = {}) {
  const distRoot = resolve(distDirectory);
  return createServer(async (request, response) => {
    response.setHeader("X-Content-Type-Options", "nosniff");
    response.setHeader("Referrer-Policy", "no-referrer");
    response.setHeader(
      "Content-Security-Policy",
      "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
    );
    if (request.method !== "GET" && request.method !== "HEAD" && !(request.method === "POST" && request.url === "/api/quarantine/release")) {
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
    if (pathname === "/api/status") {
      try {
        const [source, info] = await Promise.all([
          readFile(statePath, "utf8"),
          stat(statePath),
        ]);
        const snapshot = JSON.parse(source);
        if (
          snapshot.schema_version !== 1 ||
          !Array.isArray(snapshot.sites) ||
          !snapshot.counters
        ) {
          respondJson(response, 503, {
            error: "El estado del crawler tiene un formato no compatible.",
          });
          return;
        }
        respondJson(response, 200, {
          snapshot,
          meta: {
            read_at: new Date().toISOString(),
            modified_at: info.mtime.toISOString(),
            process_alive: processAlive(snapshot.pid),
          },
        });
      } catch (error) {
        const missing = error.code === "ENOENT";
        respondJson(response, 503, {
          error: missing
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
