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

## Transporte HTTP y DNS C04

El bloque desplegable «HTTP y DNS» muestra `snapshot.transport` cuando está disponible. Sus contadores pertenecen al transporte compartido por páginas, robots y sitemaps: peticiones iniciadas, recepción y liberación de cuerpos, uso de clientes y resolución DNS. Una petición puede fallar antes del envío por validación, DNS o espera del pool. Los documentos anteriores conservan su presentación sin inventar valores de transporte.

«Clientes creados» y «Reutilizaciones de cliente» describen clientes del pool, no conexiones TCP. Los clientes activos permanecen ocupados hasta completar o liberar el cuerpo; los disponibles pueden atender otra petición. El panel publica los límites global y por origen y distingue los timeouts de petición de los del cuerpo. Los tiempos del pool y DNS son acumulados; el timeout de petición incluye esas esperas. La caché distingue aciertos positivos, negativos, consultas agrupadas y destinos rechazados.

Las consultas DNS lógicas se muestran separadas de las consultas al resolver del sistema (`dns_blocking_inflight` y su pico). Estas últimas pueden seguir en curso hasta que termine la llamada del sistema aunque se haya cancelado la petición. Los snapshots que aún no publican esos dos campos muestran «—», sin convertir su ausencia en cero.

Los fallos de petición, cuerpos interrumpidos y rechazos de DNS no se suman a los fallos de almacenamiento ni alteran las páginas guardadas. Si se interrumpe la lectura del estado, se conserva la última observación con una advertencia. El banco aislado es quien puede medir conexiones TCP reales desde su servidor de pruebas; esas cifras no se incorporan al estado operativo.

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

El visor también acepta el informe C04 de `data/crawler-transport-bench/report.json` en una instancia separada. Distingue HTTP/HTTPS, cantidad de orígenes y cliente por petición/compartido. Publica mediana, MAD y rango de conexiones TCP reales por página, consultas DNS y descriptores, conserva la comparación de carga y ofrece el detalle por repetición. La descarga mantiene el documento completo del banco; no convierte clientes del pool en conexiones TCP.

## Verificación de navegador

```sh
npm run test:browser
```

La prueba usa el Playwright ya instalado en `frontend/node_modules/playwright` y Chromium en `/usr/bin/chromium`; se pueden sustituir mediante `CRAWLER_PLAYWRIGHT_MODULE` y `CHROMIUM_PATH`. Requiere permiso para abrir servidores locales. Comprueba aislamiento de contadores, banco deshabilitado, banco sin estado operativo, recursos, descarga, compatibilidad con snapshots antiguos, resultados C02 y vista móvil. Sus datos se crean y eliminan en un directorio temporal y sólo admite tráfico a `127.0.0.1`.

`CRAWLER_SCREENSHOT_DIR=/tmp/c01-dashboard` guarda capturas opcionales de esa prueba. Son datos sintéticos para probar la interfaz, no resultados de capacidad.

Con el crawler C04 ya iniciado, `CRAWLER_EXPECT_TRANSPORT=1 node test/browser-live.mjs` contrasta sus métricas reales con el DOM en `http://127.0.0.1:3100/`; no inicia servicios ni inyecta datos. Genera capturas y evidencia en `data/crawler-c04-dashboard-*`. Puede fijarse el PID esperado mediante `CRAWLER_EXPECTED_PID`.

`node test/browser-transport-report.mjs` abre temporalmente una instancia aislada para verificar el informe C04 real, su descarga y la presentación en escritorio y móvil. No carga ese informe en el dashboard operativo. El archivo puede sustituirse con `CRAWLER_TRANSPORT_REPORT`; la evidencia se guarda en `data/crawler-c04-benchmark-dashboard-*`.
