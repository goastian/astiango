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

La prueba usa el Playwright ya instalado en `frontend/node_modules/playwright` y Chromium en `/usr/bin/chromium`; se pueden sustituir mediante `CRAWLER_PLAYWRIGHT_MODULE` y `CHROMIUM_PATH`. Requiere permiso para abrir servidores locales. Comprueba aislamiento de contadores, banco deshabilitado, banco sin estado operativo, recursos, descarga y vista móvil. Sus datos se crean y eliminan en un directorio temporal y sólo admite tráfico a `127.0.0.1`.

`CRAWLER_SCREENSHOT_DIR=/tmp/c01-dashboard` guarda capturas opcionales de esa prueba. Son datos sintéticos para probar la interfaz, no resultados de capacidad.
