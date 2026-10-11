# Dashboard del crawler

Desde este directorio:

```sh
npm ci
npm run check
npm test
npm run build
npm start
```

El estado operativo se consulta en `/api/status` desde `CRAWLER_STATE_PATH` (por defecto, `data/crawler-local/status.json` en la raíz del repositorio). La API conserva el documento del crawler y sólo añade metadatos de lectura.

## Resultados explícitos

El panel «Solicitudes y resultados» separa `page_http_attempts` (envíos HTTP de páginas y sitemaps, incluidos reintentos; excluye robots.txt), `redirect_hops` (redirecciones seguidas) y `useful_pages` (páginas confirmadas por el almacenamiento). `attempted` conserva el presupuesto de páginas candidatas y no cuenta los saltos como nuevas páginas.

Los contadores `deferred`, `cancelled` y `storage_failed` distinguen trabajo aplazado, cancelación y fallo al guardar. Se muestran en el resumen, la distribución y el detalle del sitio; los eventos conservan sus causas. La proporción de descargas correctas usa guardadas / (guardadas + fallos permanentes + fallos de almacenamiento), sin contar aplazamientos o cancelaciones como éxito.

Los campos son aditivos en `schema_version: 1`. Los snapshots anteriores siguen disponibles y muestran «—» donde falta un contador C02. El dashboard sólo lee observaciones y no define presupuestos, pausas ni política de red.

En el crawler local, las URL aplazadas o canceladas permanecen en la frontera con una fecha mínima de reintento; no se cierra un dominio pendiente para comenzar otra ronda. La configuración explicita `network_policy = "public_only"` y `max_redirect_hops = 10`; el observador no modifica esas opciones. Cada job conserva su resultado completo en `data/crawler-local/jobs/<run_id>/*.json` usando las rutas por defecto, incluso cuando termina con fallos permanentes. El panel conserva el estado terminal del sitio y los contadores globales de toda la ejecución.

Para compilar e iniciar el servicio local y el dashboard operativo desde la raíz del repositorio, use el flujo documentado en [Crawler local y panel de trabajo](../../docs/dashboard-crawler.md). Su puerto habitual es `http://127.0.0.1:3100/`. El banco C01 utiliza un informe y un puerto separados; no hace falta habilitarlo para ver los contadores C02.

## Banco local C01

El banco se habilita explícitamente con `CRAWLER_BENCHMARK_PATH`. Sin esta variable, `/api/benchmarks` responde `404 BENCHMARK_DISABLED` y su sección no aparece. Los resultados C01 se muestran por separado, con mediana, MAD, rango, comparación del observador, etapas, recursos y procedencia. Su descarga contiene el informe C01 completo.

Después de generar `data/crawler-bench/report.json`, esta orden **desde la raíz del repositorio** abre un dashboard independiente en el puerto 3101:

```sh
CRAWLER_BENCHMARK_PATH="$PWD/data/crawler-bench/report.json" \
CRAWLER_STATE_PATH="$PWD/data/crawler-bench/no-production-state.json" \
HOST=127.0.0.1 PORT=3101 node tools/crawler-dashboard/server.mjs
```

Abrir `http://127.0.0.1:3101/#banco-c01`. El estado operativo puede estar ausente; el informe del banco continúa disponible. Esta orden no inicia ni detiene el crawler existente.

La API del banco es de sólo lectura. Un informe ausente o incompatible devuelve `503` sin afectar `/api/status`. Las fixtures y sus peticiones nunca se incorporan a los contadores operativos.

## Verificación de navegador

```sh
npm run test:browser
```

La prueba usa el Playwright ya instalado en `frontend/node_modules/playwright` y Chromium en `/usr/bin/chromium`; se pueden sustituir mediante `CRAWLER_PLAYWRIGHT_MODULE` y `CHROMIUM_PATH`. Requiere permiso para abrir servidores locales. Comprueba aislamiento de contadores, banco deshabilitado, banco sin estado operativo, recursos, descarga, compatibilidad con snapshots antiguos, resultados C02 y vista móvil. Sus datos se crean y eliminan en un directorio temporal y sólo admite tráfico a `127.0.0.1`.

`CRAWLER_SCREENSHOT_DIR=/tmp/c01-dashboard` guarda capturas opcionales de esa prueba. Son datos sintéticos para probar la interfaz, no resultados de capacidad.
