# Plan maestro de mejora y evolución de Stract/AstianGO

Actualizado: **12 de octubre de 2026**. Sustituye la planificación de agosto; se conserva el nombre del archivo para mantener las referencias existentes.

**Prioridad de ejecución: finalizar el crawler y su operación recuperable antes de abrir el siguiente componente crítico: ingesta e indexación.** Después siguen webgraph y centralidad, búsqueda y API, operación integral y frontend/producto. Las mejoras de herramientas, CI y dashboard necesarias para cerrar el crawler forman parte de ese trabajo.

El estado incorpora los commits C01–C10 y los cambios locales C11–C13 implementados y verificados el 12 de octubre de 2026. La base Git es `377a5ab8`; fuentes, binarios, configuración y resultados están ligados por SHA en la [evidencia C11–C13](crawler-c11-c13-verificacion-2026-10-12.json). URL/captura v1 se conserva y sitio/tenant usa v2 con migración y correspondencias. «Implementado», «verificado localmente» y «apto para operación prolongada» son estados distintos; este plan no declara terminado el crawler ni validado un clúster de producción.

## 1. Reglas de ejecución y definición de terminado

1. Mantener **un componente principal activo**. Dentro del crawler pueden avanzar tareas independientes, siempre que respeten las dependencias de las tablas.
2. Conservar los IDs C01–C28 del [plan de evolución del crawler](plan-evolucion-crawler-2026-10.md). Los sufijos de este documento desglosan trabajo ejecutable; no crean otra implementación de la misma capacidad.
3. P0 significa corrección, durabilidad o condición imprescindible para habilitar un modo; P1, capacidad y eficiencia necesarias para operarlo; P2, expansión posterior. La prioridad no elimina dependencias.
4. Una tarea se cierra con código, pruebas pertinentes, comparación de recursos cuando aplique, evidencia enlazada y migración/rollback cuando cambie datos o contratos. Cada puerta exige aceptar todas las tareas obligatorias de su componente; las condicionadas requieren una decisión documentada de activación o aplazamiento.
5. No abrir mejoras generales del indexador, webgraph o frontend antes de la puerta G-C. Se permiten los cambios mínimos en lectores y contratos necesarios para verificar los archivos del crawler, incluidos en C09/C21.
6. Primero se cierra el perfil de **un nodo**. El clúster C25–C26 y la extracción avanzada C28 permanecen pendientes y condicionados; no se presentan como terminados al cerrar ese perfil.
7. Si el despliegue exige varios nodos o tolerar la pérdida física del servidor, primero se acepta G-C/nodo y después G-C/clúster, que añade C25–C26 y sus pruebas **antes** de abandonar el crawler. Un ACK en disco local no demuestra esa garantía. En las fases posteriores, G-C significa la puerta correspondiente al perfil requerido.

Estados utilizados:

| Estado | Significado |
|---|---|
| Implementado | Existe código en el árbol revisado; no implica que esté publicado ni desplegado. |
| Verificado localmente | Hay resultados de pruebas o mediciones con alcance identificado. |
| Cierre pendiente | Falta evidencia, integración, migración o una condición de aceptación. |
| Pendiente | El trabajo descrito no se considera completado. |
| Condicionado | Se activa únicamente si el perfil o una medición justifica su alcance. |

Todas las casillas abiertas de las secciones de trabajo son tareas pendientes, incluso cuando amplían una implementación existente. Los responsables son **roles propuestos**, no personas ya asignadas. El responsable de cada bloque debe asignar dueño y revisor antes de iniciarlo.

## 2. Estado real y cambios que ya deben aprovecharse

### 2.1 Avances del crawler

| Entrega | Estado al 12 de octubre | Qué ya existe y debe conservarse | Qué no demuestra todavía |
|---|---|---|---|
| C01 Banco reproducible | Implementado; baseline documentado | Corpus, reconciliación HTTP/WARC, medidas de CPU/RSS y comparación del observador. Commit `2f0d92dd`; [guía](crawler-bench.md) y [baseline](crawler-bench-baseline-2026-10-11.md). | Capacidad de producción ni rendimiento universal sobre Internet. |
| C02 Semántica común | Implementado; verificación local reportada | Resultados explícitos, separación de aplazados/cancelados/fallos de almacenamiento y comportamiento independiente del panel. Commit `3bdba648`; `crawler/outcome.rs`, `runtime.rs`, `worker.rs` y `local.rs`. | Frontier transaccional por URL ni efectos idempotentes después de cualquier caída. |
| C03 Observabilidad asíncrona | Implementado; verificación local reportada | Publicación de snapshots fuera de la descarga, agregación y fallo de observación separado del estado durable. Commit `360b405f`; `crawler/observer.rs`. | Series históricas completas, alertas o control remoto auditado. |
| C04 HTTP y DNS compartidos | Implementado; verificación local reportada | Pool acotado, caché DNS, validación de destinos y política `public_only`. Commit `00feda86`; `crawler/transport.rs`. | Que toda ruta futura pueda omitir pruebas de red, redirecciones o cortesía. |
| C05 Admisión y cuerpos | Implementado; verificación local reportada | Límites de bytes y reservas, admisión por respuesta y tiempos separados para páginas, robots, feeds y sitemaps. Commit `e30dae53`; `crawler/body.rs`. | Límite del RSS completo ni presupuesto agregado de un árbol de sitemaps. |
| C06 Extracción compartida | Implementado; verificación local reportada | Parseo compartido de enlaces/metadatos, cuotas y banco de CPU/asignaciones. Commit `b7683215`; `crawler/extraction.rs`. | Que todos los parsers posteriores de indexación estén optimizados. |
| C07 Pipeline acotado | Implementado; verificación local reportada | Pools de parseo, almacenamiento, control y preparación de red; presupuestos por elementos/bytes y métricas de espera. Commit `8289b91e`; `crawler/pipeline.rs`. | Scheduler que libera el turno de un origen durante la cortesía ni governor por cgroup. |
| C08 Spool durable | **Implementado y verificado localmente** | Commit `33e0ab5b`: writer común local/batch, journal, segmentos, ACK local, recuperación, publicación S3 y referencias de retención. [Contrato](crawler-c08.md), [resultados](crawler-c08-resultados-2026-10-12.md), [JSON](crawler-c08-resultados-2026-10-12.json) y [verificación final](crawler-c08-verificacion-2026-10-12.json). | Captura HTTP exacta C09, deduplicación transaccional C10, pérdida física de disco o cierre operacional completo. |
| Dashboard y runner | Implementados; consolidación verificada localmente | Un panel con estado operativo y banco aislado, puerto documentado `3101`, configuración conservada entre arranques y reinicio controlado. [README](../tools/crawler-dashboard/README.md) y [verificación](crawler-dashboard-central-verificacion-2026-10-12.json). | Centro integral con catálogo de páginas, RBAC, auditoría y comandos distribuidos. |

Las rutas `crawler/...` de esta tabla son relativas a `crates/core/src`. Las suites y ejemplos específicos están junto a cada implementación y bajo `scripts/`.

**Pendiente documental:** el plan de evolución ya declara C01–C08 implementados, pero conserva hallazgos iniciales desactualizados y enlaces a informes C02–C07 que no están presentes en este checkout. El JSON del baseline C01 también requiere consolidación. C08 sí dispone de informe final con pruebas de contratos, binario normal, reinicio y dashboard real. C08.1 debe recuperar la evidencia histórica restante y reconciliar las guías; no reabre la implementación C08. No se consideran reejecutadas esas pruebas por actualizar este documento.

### 2.2 Resultado medido de C08 y alcance de la mejora

El informe C08 contiene **39 mediciones y 60 casos de recuperación**, con tres repeticiones por perfil y S3 local. El banco verificó 72 objetos por SHA-256 y no dejó multiparts pendientes. No fue una prueba de estrés externa. La [verificación final](crawler-c08-verificacion-2026-10-12.json) registra además contratos WARC/indexador/webgraph, pruebas Rust/Node/runner, compilación, banco y métricas operativas en escritorio/móvil; el reinicio conserva frontier, semillas y WARC históricos.

| Comparación de archivo sobre el mismo corpus | Resultado publicado |
|---|---|
| Baseline por página gzip 9 frente a segmentos gzip 3, grupos de 16 | Mediana de 79,70 a 258,43 páginas/s; +224,23% como cociente de medianas. |
| CPU de archivo | Reducción del 66,06%; los bytes comprimidos aumentan 4,48%. |
| Sincronizaciones y archivos WARC | Reducciones emparejadas del 91,02% y 99,22%, respectivamente, en grupos de 16. |
| RSS máximo del banco | 36,96 MiB en escritura y 46,40 MiB en publicación; no representan el RSS del crawler completo. |
| Commit individual, segmentos gzip 9 | Variación emparejada mediana de −1,78% en páginas/s y 2,73% más sincronizaciones. Agrupar tiene un efecto propio. |

La ganancia emparejada mediana de throughput publicada es 244,81%; utiliza otro cálculo y no debe intercambiarse con el 224,23% anterior. El crawler local de dos trabajadores no garantiza grupos de 16. C07 ya había acotado el buffer batch a 64 MiB: no se atribuye a C08 toda la mejora frente al buffer histórico de aproximadamente 1 GiB. Las cifras proceden de los [resultados C08](crawler-c08-resultados-2026-10-12.md).

### 2.3 Estado de los demás componentes

| Componente | Base existente | Deuda confirmada o cierre requerido | Turno |
|---|---|---|---|
| Frontier y scheduler | Estado local persistido, reanudación y resultados explícitos | JSON completo, progreso por dominio, identidad y transacciones por URL, elegibilidad sin mantener turnos dormidos. | Dentro del crawler. |
| Coordinator/router batch | Reparto de jobs mediante cola de archivos | `FileQueue::pop` consume antes del ACK; leases, reentrega y failover pendientes. | C25 si se habilita clúster; restringir uso mientras tanto. |
| Live crawler | Motor compartido, marca tras `DurableSuccess`, ACK de índice con timeout y permisos retenidos hasta confirmar | Cursor/TTL/errores y reintentos/outbox durables pendientes. Su ACK no es el spool de C08. | C20 dentro del cierre del crawler. |
| Ingesta/indexador/Tantivy | Lectura WARC, índice local y serving por shards | WARC completo en memoria, stacks de 160 MB por hilo, writer de 1.000.000.000 bytes, errores omitidos y dedup no persistente. | **Siguiente componente crítico tras G-C.** |
| Webgraph/centralidad | Grafo offline, algoritmos y finalización limitada a cuatro tareas | Presupuesto global de RAM/I/O, writer de 1.000.000.000 bytes, generaciones y cálculo incremental. | Después de ingesta/indexación. |
| Search/API | Shards/réplicas, timeouts/retries internos y límite opcional de concurrencia HTTP | Límites coherentes de extremo a extremo, readiness, cancelación, publicación de generaciones y SLO medidos. | Después de webgraph. |
| Centro operativo | Dashboard específico del crawler | Integración entre componentes, catálogo URL→WARC→índice, comandos y auditoría. | Mínimo crawler ahora; centro integral después del serving. |
| Toolchain y CI | Workspace/lockfile y workflow existentes | Sin pin Rust, miembro `client-wasm` duplicado, debug en release, `npm install` y tareas de pruebas repetidas. | Solo habilitadores del crawler ahora; resto por componente. |
| Frontend, WASM y herramientas | Aplicación pública y utilidades de ranking/anotación | Build reproducible, presupuestos de carga, calidad/accesibilidad y mantenimiento de dependencias. | Después de los componentes críticos. |

Fuentes de estas deudas: [indexador](../crates/core/src/entrypoint/indexer/mod.rs), [worker de indexación](../crates/core/src/entrypoint/indexer/worker.rs), [lector WARC](../crates/core/src/warc.rs), [ingesta compartida](../crates/core/src/entrypoint/mod.rs), [webgraph](../crates/core/src/entrypoint/webgraph.rs), [workspace](../Cargo.toml) y [CI](../.github/workflows/ci.yaml).

## 3. Arquitectura y contratos que gobiernan el plan

### 3.1 Flujo objetivo del perfil de un nodo

```text
semillas / planner / feeds / sitemaps
                 |
       frontier transaccional por URL
                 |
     scheduler por origen y elegibilidad
                 |
  admisión -> DNS/HTTP -> decode/extracción
                 |
      spool local durable por segmentos
                 |
      publicación opcional verificada
                 |
   lectores acotados y ACK de consumidores
                 |
  índice / webgraph por generación -> búsqueda

observación asíncrona -> dashboard y métricas
```

La frontier, el scheduler y el contrato completo de captura son trabajo pendiente. Se conserva el transporte, extracción, pipeline y spool ya construidos.

### 3.2 Separar estados y garantías

- **Admitida durable:** la frontier puede recuperar la URL y su elegibilidad después de reiniciar.
- **Archivada durable local:** C08 confirmó datos y journal en el dispositivo local. La confirmación depende del cumplimiento de las sincronizaciones por el sistema de almacenamiento.
- **Publicada remota:** el segmento tiene confirmación de publicación y verificación correspondiente; una cola S3 pendiente no cambia el ACK local.
- **Aceptada para indexación:** el consumidor aplicó las reglas de calidad, duplicados y exclusiones.
- **Indexada/publicada en búsqueda:** pertenece a una generación validada y, en el segundo caso, activa en serving.

`useful_pages` del dashboard actual cuenta páginas confirmadas por almacenamiento. **No equivale a páginas aceptadas ni publicadas en el buscador.** Los futuros indicadores deben conservar esa separación, con versión de esquema y compatibilidad con snapshots antiguos.

C08 ofrece entrega al menos una vez: cancelar después de encolar puede dejar una captura durable cuyo ACK no vio el llamante. C10 reconcilia esa ventana mediante `fetch_id`, recibos y outbox, incluyendo los descubrimientos. Ninguna de las dos etapas promete una transacción distribuida universal.

C09 escribe captura WARC/1.1 con payload recibido, cabeceras coherentes, transformaciones explícitas y decisiones de robots. Mantiene lector WARC/1.0 y descriptores anteriores sin modificar los archivos históricos. Live-index conserva su confirmación de aplicación; compartir fetcher no iguala sus garantías. Contrato, migración y rollback en [crawler-c09-c10.md](crawler-c09-c10.md).

La retención actual mantiene `consumer:archive` y GC desactivado. Ningún consumidor libera automáticamente esa referencia. Además, el GC exige estado `Published`: los segmentos del modo local, en estado `Local`, no son elegibles aunque se liberen referencias. La cuota puede llenarse: C21.3 debe definir una política local verificable con archivo/backup y recuperación, no borrar segmentos para mantener una cifra de velocidad.

### 3.3 Recursos actuales y métricas de éxito

Valores de los ejemplos [local](../configs/crawler/local.toml) y [batch](../configs/crawler/crawler.toml), no capacidad certificada:

| Recurso | Configuración observada | Acción pendiente |
|---|---|---|
| Trabajo local | Dos trabajadores; demora mínima de 10 s; frontier máxima de 10.000. | Desacoplar turno del origen y espera; sustituir límite en memoria por almacenamiento escalable. |
| Cuerpos | Hasta ocho cuerpos; reserva global de 512 MiB; HTML/feed/sitemap de hasta 32 MiB recibidos y decodificados. | Calibrar con RSS real y perfil pequeño; mantener reserva durante el uso. |
| Pipeline | Ocho documentos y 256 MiB de entradas; pools de parseo/almacenamiento/control/red separados. | Medir colas, DOM, copias y memoria ajena a las reservas. |
| Transporte | 128 slots globales, cuatro por origen, 1.024 entradas DNS. | Ajustar por perfil y FD; un slot de cliente no equivale a una conexión TCP medida. |
| Spool | Segmentos de 128 MiB, rotación a 300 s, gzip 3, cuota de 8 GiB y hasta 256 segmentos; commits de hasta 16 registros/10 ms. | Dimensionar retención y tiempo hasta llenar cuota según bytes útiles y velocidad de consumidores/publicación. |
| Upload | Buffer propio de hasta 5 MiB por parte, envío secuencial. | Incluir copias del SDK y RSS de proceso; validar proveedor objetivo. |

Las reservas de cuerpos, pipeline y colas pueden representar el mismo documento en distintas etapas; no se suman ciegamente ni se usan como límite de RSS. Los stacks reservados tampoco son memoria residente medida.

```text
páginas archivadas por CPU-hora = ACK locales únicos / CPU-horas de adquisición
páginas indexables por CPU-hora = documentos aceptados únicos / CPU-horas del pipeline
bytes por página útil          = bytes de red o archivo / documentos útiles únicos
coste por millón útil          = coste observado / documentos útiles únicos × 1.000.000
tiempo hasta cuota de spool     = espacio utilizable / crecimiento neto de bytes por segundo
```

CPU-horas se obtiene del tiempo de CPU, no de multiplicar duración por todos los núcleos visibles. Hasta disponer del consumidor se publica la métrica de archivo y se deja la de indexables sin valor. Cada informe debe indicar denominador, corpus, hardware, límites, versión, configuración y dispersión.

## 4. Orden de componentes y puertas de avance

| Orden | Componente y responsable propuesto | Entrega de cierre | Puerta para empezar el siguiente |
|---|---|---|---|
| 1 | Crawler — Rust/crawler + revisión de almacenamiento y operación | C01–C24 y C27 en el alcance de un nodo; evidencia y operación prolongada. | G-C de la sección 6. |
| 2 | Ingesta e indexación — Rust/data + revisión de búsqueda | Consumo recuperable, dedup exacta, recursos acotados y generaciones verificables. | G-I: misma entrada, documentos reconciliados, reanudación y rollback de artefactos. |
| 3 | Webgraph y centralidad — Rust/graph | Generaciones compatibles, checkpoints y procesamiento incremental medido. | G-W: grafo y scores reproducibles dentro del presupuesto. |
| 4 | Búsqueda distribuida y API — Rust/search + operación | Serving bajo SLO, sobrecarga controlada y activación/rollback de generaciones. | G-S: carga y fallo de réplica probados sin publicación parcial. |
| 5 | Operación integral — backend/control + frontend/operación | Catálogo entre componentes, órdenes auditadas, alertas y recuperación. | G-O: URL trazable, control idempotente y restore probado. |
| 6 | Frontend, calidad y funciones — frontend/producto + búsqueda | Interfaz rápida, ranking evaluado y pilotos con coste explícito. | G-F: presupuestos de carga y calidad medidos. |

No se fija otra secuencia de 30/60/120 días sin conocer equipo y resultados. Se trabaja en ciclos revisables de hasta dos semanas; una tarea grande se divide antes de entrar al ciclo. El tiempo de soak de C27 es una condición de aceptación, no una estimación de desarrollo.

## 5. Trabajo del crawler hasta su cierre

### Bloque A — Consolidar evidencia, ampliar eficiencia y definir la captura correcta

**Objetivo:** conservar C08 y C09 verificados; las mediciones adicionales de C08 permanecen pendientes. C08.2 y C09.1–C09.3 se registran completados con la evidencia disponible; C08.3–C08.4 son mejoras adicionales. Responsable: Rust/almacenamiento; revisión: crawler y consumidores.

| ID / prioridad | Tarea concreta | Dependencia | Criterio de aceptación |
|---|---|---|---|
| C08.1 · P0 | [ ] Consolidar evidencia histórica C01–C07 y referencias de C08; corregir hallazgos/enlaces desactualizados en evolución, despliegue y dashboard. | Estado actual | Cada estado de cierre enlaza evidencia disponible; archivos faltantes recuperados o pruebas regeneradas; commit y modificaciones locales identificados. No reconstruir cifras sin sus resultados. |
| C08.2 · P0 | [x] Verificar spool, binario normal, recuperación, contratos de consumidores y dashboard real. | C01–C07 e implementación C08 | [Verificación final disponible](crawler-c08-verificacion-2026-10-12.json): banco y contratos aprobados, binario sin feature de benchmark, reinicio con datos/semillas/frontier conservados y vistas operativas verificadas. Mantener estas regresiones al modificar C09/C10/C21. |
| C08.3 · P1 | [ ] Medir tamaño real de grupos, espera p95/p99 del ACK, fsync/página, CPU y bytes con 1/2/múltiples productores. Validar S3 objetivo si se utilizará. | C08.2 | Informe distingue ganancia de gzip y agrupación; publica coste de commit individual; reintento conserva pendientes; checksum y recuperación verificados en el backend habilitado. |
| C08.4 · P1 | [ ] Perfilar `Spool::refresh` y recuperación con 1/64/256/4.096 segmentos en un banco con cuotas ajustadas; evaluar contabilidad incremental de bytes/estados/referencias. | C08.2 | Publicar CPU/I/O por commit y tiempo/RSS de arranque al crecer el archivo. Si el barrido por grupo domina, sustituirlo conservando reconciliación en recuperación y cuotas exactas ante fallos; si no, documentar el umbral para intervenir. |
| C09.1 · P0 | [x] Versionar contrato de captura: URL solicitada/final, identidad de captura, status, cabeceras, IP validada, redirects, fecha, payload y transformaciones. | C02, recibos C08 | Fixtures de compresión, charset, redirects y respuestas vacías; hashes y cabeceras corresponden a la representación almacenada. No llamar captura exacta al texto actual. |
| C09.2 · P0 | [x] Implementar writer/reader compatibles y transportar decisiones de `X-Robots-Tag`, meta robots y exclusiones al consumidor. | C09.1 | Lector independiente verifica los registros; corpus antiguo legible; versión desconocida falla de forma explícita; noindex excluye del índice sin confundirse con robots de descarga. |
| C09.3 · P0 | [x] Documentar actualización primero de lectores y después de writer, referencias por registro/offset y rollback. | C09.2 | Reproducción completa de archivo antiguo/nuevo; downgrade no interpreta silenciosamente formatos desconocidos; no se destruyen originales. |

**Puerta A:** archivo recuperable y contrato verificable. C08 no se cierra atribuyéndole capacidades de C09 o C10.

### Bloque B — Frontier por URL, identidad y deduplicación

**Objetivo:** no perder progreso y evitar cómputo/red repetidos. Responsable: Rust/crawler; revisión: persistencia.

El prerrequisito de identidad de C10 está cubierto por el contrato conservador v1 de C09–C10: URL sin fragmento, parámetros conservados y origen HTTP explícito. C11.1 está completado: sitio/tenant v2, migración con correspondencias y URL/captura v1 conservadas. C12/C13 tienen [verificación local](crawler-c11-c13-resultados-2026-10-12.md). Evidencia de C09–C10 en [crawler-c09-c10-verificacion-2026-10-12.json](crawler-c09-c10-verificacion-2026-10-12.json).

| ID / prioridad | Tarea concreta | Dependencia | Criterio de aceptación |
|---|---|---|---|
| C11.1 · P0 | [x] Separar URL, origen HTTP, host, dominio registrable y tenant; versionar normalización conservadora. | C02; acordar con C09.1 | IDN, IPv6, puertos, escapes, URLs firmadas y sitios multiusuario preservan recursos distintos. No eliminar/reordenar parámetros arbitrarios. |
| C10.1 · P0 | [x] Diseñar e implementar tablas de URL/estado/elegibilidad/reserva/commit/outbox con transacciones y caché acotada; medir el motor embebido elegido. | C11.1, C09.1 | Prueba 100.000→1.000.000 URLs sin crecimiento lineal del RSS; registro durable recuperable y consultas de próxima tarea sin barrer todo el historial. |
| C10.2 · P0 | [x] Migrar snapshots JSON con copia conservada, versión, conteos y reanudación de la migración. | C10.1 | URLs admitidas, aplazadas y pendientes reconciliadas antes/después; caída a mitad de migración no mezcla autoridades ni elimina el original. |
| C10.3 · P0 | [x] Integrar reserva→captura→recibo→commit y outbox de enlaces/resultados; reconciliar huérfanos por identidad de captura. | C08.2, C09.3, C10.2 | Caída en cada transición no pierde URL admitida ni descubrimientos; los reintentos no duplican efectos finales. HTTP repetido queda identificado. |
| C16.1 · P0 | [ ] Corregir y medir fórmula/capacidad/hash del Bloom con prueba estadística, límite de RAM y formato versionado. | C01 | Memoria calculada frente a asignada y tasa efectiva de falsos positivos publicadas; no basta el parámetro nominal 5%. Migración explícita de filtros. |
| C16.2 · P0 | [ ] Hacer URL/hash exactos persistentes la autoridad de dedup; Bloom solo como acelerador. | C10.3, C11.1, C16.1 | Bloom saturado no elimina URLs legítimas; mismo contenido/manifest reprocesado mantiene identidad lógica; medir consultas evitadas, CPU, RAM y ahorro de descargas. |

El constructor `BytesBloomFilter::new(10_000_000_000, 0.05)` del indexador declara cardinalidad estimada, no diez mil millones de bits. La fórmula local requiere revisión; corregirla sin reducir/calibrar la capacidad puede aumentar fuertemente la memoria. No conservar el dimensionamiento antiguo como objetivo. Ver [implementación Bloom](../crates/bloom/src/lib.rs).

**Puerta B:** progreso y dedup sobreviven a reinicios; cualquier URL omitida tiene una razón auditable.

### Bloque C — Scheduler y descubrimiento que aprovechen el cómputo

**Objetivo:** trabajar sobre otros orígenes durante las esperas y reducir tráfico inútil sin alterar cortesía. Responsable: Rust/crawler.

| ID / prioridad | Tarea concreta | Dependencia | Criterio de aceptación |
|---|---|---|---|
| C13.1 · P0 | [x] Cachear robots por esquema/host/puerto, con TTL, límite, consultas simultáneas agrupadas y clasificación de fallos. | C04, C11.1 | HTTP/HTTPS no mezclan reglas; cien misses simultáneos causan una descarga; expiración y recuperación con reloj controlado; corpus de matching y redirecciones verificable. |
| C12.1 · P1 | [x] Implementar colas ready/deferred por `next_eligible_at`, turnos pequeños y puerta de cortesía común para páginas/robots/sitemaps. | Puerta B, C13.1, C05–C07 | Ningún permiso HTTP queda retenido durante una pausa de cortesía; sitios lentos/grandes no bloquean otros; cola lista tiene espera acotada bajo carga estable. |
| C12.2 · P1 | [x] Comparar scheduler con jobs largos a iguales CPU, conexiones, corpus y política remota. | C12.1 | Meta de ≥3× throughput en fixture de muchos orígenes con esperas; publicar resultado aunque no se alcance. Cero solapamientos/intervalos prohibidos; no extrapolar a Internet. |
| C14.1 · P0 | [ ] Persistir reintentos, fecha mínima, intentos y presupuesto total; conservar `Retry-After` y clasificar 408/429/5xx/red/TLS. | C10.3, C12.1 | Reiniciar no adelanta un retry; presupuesto agotado deja razón terminal; 404/410 no entran en bucle rápido y TLS inválido nunca desactiva validación. |
| C14.2 · P1 | [ ] Añadir recuperación gradual de velocidad, jitter y circuit breaker por origen. | C14.1 | Un 429 no penaliza para siempre; no hay tormenta tras recuperación; mejora de utilización con igual o menor carga indebida al sitio. |
| C15.1 · P1 | [ ] Leer feeds/sitemaps incrementalmente y limitar bytes, entradas, profundidad y documentos agregados por origen/ciclo. | C05, C10.3, C12.1 | 50.000 URLs dentro del presupuesto; ciclos terminan con causa; XML inválido no aborta proceso; repetir feed no duplica candidatos. |
| C17.1 · P1 | [ ] Aplicar cuotas por patrones de URL, calendario, facetas y contenido repetido; cuarentena reversible y muestreo. | C16.2, C15.1 | Expansión adversarial acotada; corpus legítimo conserva cobertura acordada; publicar ahorro de fetch y falsos descartes. |
| C18.1 · P1 | [ ] Persistir validadores HTTP, emitir peticiones condicionales y representar 304/revisitas referenciando el payload anterior. | C09.3, C10.3, C14.1, C16.2 | Sin cambios no duplica cuerpo ni parseo; cambios producen versión nueva; GC conserva referencias; reinicio mantiene estado. |
| C18.2 · P1 | [ ] Programar recrawl por cambios, valor, coste y clase de frescura; contrastar con intervalo fijo. | C18.1 | Meta ≥30% menos bytes en cohorte elegible, con lag p95 de cambios dentro del objetivo acordado; muestreo detecta validadores defectuosos. |
| C19.1 · P0 | [ ] Corregir agrupación del planner, validar TOML/límites antes de escribir y añadir checkpoints. | C11.1, C10.2 | Ranks conocidos generan grupos correctos; cero colas/fracciones inválidas fallan al inicio; caída reanuda sin borrar salida válida. |
| C19.2 · P1 | [ ] Ordenar candidatos con memoria acotada y combinar valor/coste/frescura con cuota explícita de exploración. | C15.1–C18.2, C19.1 | 10× candidatos no exige 10× RSS; comparar documentos útiles, diversidad y coste a igual presupuesto; semillas conservan procedencia. |

**Puerta C:** mayor utilización del nodo con cobertura preservada y red/CPU por documento útil medidas. Alcanzar throughput por reducir cortesía o descartar páginas válidas no cuenta como mejora.

### Bloque D — Integración, perfiles, dashboard y artefacto

**Objetivo:** operar con límites completos, consumidores compatibles y datos confiables. Responsable: Rust/crawler; apoyo específico de data, dashboard y operación.

| ID / prioridad | Tarea concreta | Dependencia | Criterio de aceptación |
|---|---|---|---|
| C20.1 · P0 | [ ] Corregir cursor, futuros pendientes, errores y TTL del live crawler con reloj controlado. | C02 | Más de cien URLs se recorren hasta fin sin bucles; `Pending` no reinicia/trunca el stream; expiración y fallos conservan trabajo. |
| C20.2 · P0 | [ ] Admitir antes de spawn y persistir reintentos/outbox; compartir elegibilidad conservando marca tras `DurableSuccess`, timeout de ACK y permisos ya implementados. | C20.1, C10.3, C12.1 | Caída del índice o fetch recuperable incluso después de reiniciar, sin falso «actualizado» ni crecimiento ilimitado; diferencias entre frescura, archivo y publicación documentadas. No exige activar live-index en crawler-only. |
| C21.1 · P0 | [ ] Crear lectura WARC incremental común para consumidores actuales, con límites por registro, errores explícitos y pruebas de formato. | C09.3, C16.2 | Segmento mayor que el presupuesto se procesa sin cargarlo entero; corrupción se informa/cuarentena, no desaparece mediante `.flatten()`. |
| C21.2 · P0 | [ ] Añadir checkpoint/ACK de consumo y protocolo de referencias de retención; prueba end-to-end mínima de indexador y webgraph. | C10.3, C21.1 | Reinicio a mitad de segmento reanuda; manifest repetido no duplica efectos; consumidor caído acumula hasta cuota y frena admisión. GC sigue desactivado hasta probar referencias y restore. |
| C21.3 · P0 | [ ] Definir e implementar retención para archivo local y remoto, liberación de referencias y backup; resolver explícitamente la condición `Published` del GC local. | C08.2, C21.2 | Ningún payload requerido por consumidor/304/revisita/outbox/backup se elimina; rotación y reclamación sobreviven caída. Para cada perfil se prueba restauración del archivo retenido y actuación al llenar cuota; GC solo se habilita con esas pruebas. |
| C22.1 · P0 | [ ] Validar configuración efectiva y ejemplos; rechazar claves desconocidas, valores imposibles y seeds/advertise inválidos. | C09.1 | Corregir claves antiguas de `crawler.toml` y planner; todos los ejemplos habilitados pasan prueba de carga; errores ocurren antes de rastrear. |
| C22.2 · P1 | [ ] Fijar builder/toolchain/lockfile y producir artefacto con build ID, símbolos externos y dependencias mínimas; aislar crawler por crate o features verificables. | C22.1; contratos estabilizados | Build limpio y arranque reproducibles; árbol de dependencias justificado; medir tiempo de compilación/tamaño/RSS. Resolver o aislar la cadena nativa que requiere `gnu17`; no confundir menos dependencias con ahorro runtime probado. |
| C22.3 · P0 | [ ] Integrar checks Rust/Python/dashboard y bancos críticos en CI; evitar pruebas duplicadas y conservar artefactos de aceptación. | C08.1, C22.2 | Una ejecución identifica versión, lockfiles, submódulos, corpus y resultados; `npm ci` para dashboard; regresión funcional o de formato bloquea entrega. |
| C23.1 · P1 | [ ] Definir perfiles pequeño/servidor con cuotas de CPU, RSS, FD, cuerpos, DOM, cachés, red, disco/inodos y spool; respetar cgroups. | C05–C08, C12.1 | Perfil pequeño objetivo de 512 MiB o ajuste documentado con medición; cuota de dos CPU no crea pools según todos los CPU del host; no hay OOM. |
| C23.2 · P1 | [ ] Ajustar admisión por presión local y consumidores, con histéresis, mínimos/máximos y modo manual. | C23.1, C21.3 | CPU limitada, red lenta y poco disco reducen admisión sin pérdida de ACK; recuperación no oscila; razones visibles; mejora frente a perfil fijo publicada. |
| C24.1 · P1 | [ ] Ampliar el dashboard existente con tasas/edades por etapa, recursos, retraso de consumidor, pendientes, exclusiones y coste. | C10.3, C23.1 | Un operador diferencia cortesía, red, CPU, cuota y parada; cifras reconcilian con frontier/journal/manifiestos; distingue ejecución actual y acumulado del spool. |
| C24.2 · P1 | [ ] Añadir consultas paginadas, histórico agregado y métricas exportables; medir caché/polling/SSE con diez clientes. | C03, C24.1 | Misma semántica con/sin panel; objetivo <2% CPU adicional de telemetría y regresión de throughput ≤5% en runner estable; etiquetas sin URL/UUID/dominio ilimitados. |
| C24.3 · P0 | [ ] Añadir health/readiness y alertas mínimas: estado obsoleto, spool en riesgo, consumidor/publicación detenidos y error durable. | C08.2, C21.2, C24.1 | Cada alerta tiene umbral, deduplicación y runbook probado; caída del dashboard no detiene ni cambia el crawl. Mantener separados banco y producción. |

C21 cierra compatibilidad, límites de lectura y confirmación necesarios para el crawler. La optimización completa de tokenización, writer Tantivy, merges y generaciones pertenece al componente siguiente. C22 no obliga a actualizar todos los majors del monorepo simultáneamente.

### Bloque E — Operación prolongada y cierre del crawler

**Objetivo:** demostrar recuperación y estabilidad del perfil que se va a operar. Responsable: operación + Rust/crawler.

| ID / prioridad | Tarea concreta | Dependencia | Criterio de aceptación |
|---|---|---|---|
| C27.1 · P0 | [ ] Versionar servicio/paquete, usuario, cuotas, logs, arranque y drenaje; cubrir todos los procesos del perfil habilitado. | C22.3, C23.1 | `SIGTERM`, reinicio y actualización tienen deadline y resultado visibles; reanudar conserva pendientes; rollback no borra ni interpreta mal datos. |
| C27.2 · P0 | [ ] Ejecutar matriz de fallos de proceso, disco/inodos, DNS/red, S3, consumidor y corrupción; backup/restore en entorno limpio. | Puertas A–C, C21.3, C27.1 | Cero ACK perdidos bajo la garantía declarada; se recuperan frontier, referencias, archivos y configuración; fallos fuera de garantía quedan explícitos. |
| C27.3 · P0 | [ ] Ejecutar soak de 24 h y después 7 días con carga representativa y límites del perfil. | C27.2, C23.2, C24.3 | RAM/cachés/FD acotados, crecimiento de disco explicado por retención, cero pérdida silenciosa y cero violaciones de cortesía; degradaciones y reintentos reconciliados. |
| C27.4 · P0 | [ ] Publicar expediente G-C/nodo: capacidad, recursos, costes, fallos, configuración aprobada, evidencia y runbooks. | C27.3 y restantes tareas obligatorias de un nodo | Revisión técnica acepta G-C/nodo sin P0/P1 abiertos ni evidencias sustituidas por promesas. I01 espera además G-C/clúster si ese perfil es requerido. |

### Bloque condicionado — Clúster y cobertura avanzada

| ID / prioridad | Tarea concreta | Cuándo entra | Aceptación |
|---|---|---|---|
| C25.1 · P0 antes de clúster | [ ] Sustituir consumo irreversible de jobs por leases durables, ACK/NACK, vencimiento, fencing y propiedad por origen; router tolerante a fallos. | Capacidad o disponibilidad requieren varios nodos. | Reentrega sin perder trabajo; reservas/commits antiguos se rechazan; takeover respeta solicitudes en vuelo y cooldown bajo límites declarados de reloj/pausas. No prometer exactamente una petición HTTP durante particiones. |
| C25.2 · P0 antes de clúster | [ ] Limitar/autenticar protocolo interno, separar bind/advertise y probar versiones mixtas. | Antes de exponer servicios entre nodos. | Mensajes sobredimensionados rechazados antes de reservar; nodo no autorizado no altera colas; compatibilidad/rollback verificados. |
| C26.1 · P1 | [ ] Medir 1/2/4 nodos, rebalanceo y recuperación del plano de control y de WARC confirmado. | G-C/nodo y C25. | Objetivo 2→4 ≥1,7× throughput útil a igual carga por nodo; partición sin quorum no confirma trabajo replicado; RPO/RTO probados, incluido disco perdido. |
| C28.1 · P2 | [ ] Evaluar PDF y JavaScript selectivo en cola aislada con cuotas y procedencia. | Tras componentes críticos; pérdida de cobertura medida. | Incremento de contenido útil justifica CPU/RAM/red/coste; no degrada HTML; no se activa navegador por cada página ni para eludir restricciones. |

## 6. Puertas G-C para declarar terminado el crawler

**G-C/nodo** se acepta con las ocho comprobaciones siguientes. No depende de C25–C26:

- [ ] C01–C08 tienen estado y evidencia reconciliados; C09–C24 y C27 están aceptados en el alcance anterior.
- [ ] Cada URL admitida durable termina, permanece pendiente o tiene exclusión explícita; reinicios no la hacen desaparecer.
- [ ] Cada ACK local apunta a registro recuperable; cada publicación remota declarada apunta a objeto verificado. Se distinguen garantías ante proceso, energía y dispositivo.
- [ ] Writer y consumidores leen formatos históricos/nuevos; checkpoint y referencias permiten reanudar sin pérdida ni borrado prematuro.
- [ ] Cero bypass de política de red/robots/cortesía en corpus adversarial; retries y backoff sobreviven reinicio.
- [ ] Perfil documentado cumple CPU/RSS/FD/red/disco y conserva margen; diez clientes de dashboard no alteran decisiones ni exceden su presupuesto.
- [ ] Capacidad y mejora se expresan con CPU-horas, bytes y páginas útiles; cobertura y frescura se comparan junto al throughput.
- [ ] Soak de 24 h y 7 días, backup/restore y rollback aprobados; no quedan errores persistentes sin razón ni propietario.

**G-C/clúster**, solo si ese perfil es requerido, añade:

- [ ] G-C/nodo aceptado, C25–C26 cerrados y recuperación/propiedad/cortesía verificadas bajo el modelo de fallos declarado. Hasta entonces el modo batch distribuido sigue restringido y no se avanza a indexación como componente principal.

Al cerrar G-C se congela una versión de referencia del crawler. Su mantenimiento correctivo continúa, pero el trabajo principal pasa a ingesta/indexación. Una mejora opcional de crawling no desplaza automáticamente el siguiente componente.

## 7. Segundo componente crítico: ingesta e indexación

**Inicio:** después de G-C. **Responsable:** Rust/data. Reutiliza C09/C16/C21; no se reconstruye otro lector ni otra autoridad de dedup.

| ID / prioridad | Trabajo | Dependencia | Aceptación y mejora de recursos |
|---|---|---|---|
| I01 · P0 | [ ] Medir ingesta, decode, extracción, tokenización, writer, commit y merge sobre WARC fijos; usar centralidades, modelos y grafo existentes o fixtures identificados por hash. | G-C | CPU-ms/documento, documentos/s, RSS, I/O y bytes de índice; corpus pequeño y grande con tres repeticiones. Baseline ejecutable antes de W06, con `host_centrality_store_path` y demás entradas declaradas. |
| I02 · P0 | [ ] Extender checkpoint/idempotencia de C21 al lote completo; propagar errores de descarga y registros; cuarentena con causa. | I01 | Archivo ausente/corrupto no genera «éxito» parcial silencioso; rerun completa pendientes sin duplicados lógicos; conteos de entrada/salida/rechazo reconciliados. |
| I03 · P1 | [ ] Presupuestar todos los writers, buffers y pools conjuntamente; hacer configurables concurrencia y stacks tras medir necesidad. | I01, I02 | RSS dentro de cuota al variar tamaño WARC y CPU visibles; retirar dependencia de 160 MB/hilo y 1 GB/writer como valores universales. Meta ≥30% menos RSS pico frente a baseline comparable. |
| I04 · P1 | [ ] Perfilar asignaciones/copias, extracción/tokenización y reutilización de buffers; eliminar recomputación por documentos repetidos. | I01, C16.2 | Meta ≥20% menos CPU-ms por documento aceptado en la etapa intervenida; mismo texto/tokens/decisiones en corpus y sin pérdida de calidad. |
| I05 · P1 | [ ] Escalonar merges, commits y compactación con cuotas de I/O y espacio temporal; comparar con la política actual que fuerza un segmento por WARC. | I03 | Disco lleno falla antes de corromper; ingestión y serving conservan sus presupuestos; publicar amplificación de escritura y pico de espacio temporal. |
| I06 · P0 | [ ] Crear manifest de generación con fuentes, hashes, config, esquema, modelo y artefactos; validar conteos y consultas conocidas. | I02–I05 | Generación incompleta no se marca lista; repetir input produce contenido lógico equivalente; cada resultado puede atribuirse a una versión. |
| I07 · P0 | [ ] Preparar generación nueva, conservar anterior y probar selección/rollback de artefactos sin reconstrucción. | I06 | Activación de prueba cambia manifest íntegro; interrupción conserva una generación válida. Serving público se integra en S04. |
| I08 · P1 | [ ] Auditar divergencia del fork Tantivy, compatibilidad on-disk y actualización por lotes independientes. | I06, I07 | Inventario de parches y pruebas de lectura/consulta; migración por generación si cambia formato; benchmark y ranking sin regresión no explicada. |

**G-I:** lotes de 32 y después 100 WARC se procesan bajo cuota, con recuperación, errores reconciliados, dedup exacta y generación verificable/reversible. Se conserva evidencia de CPU, RAM, espacio y tiempo por documento antes de pasar al grafo.

## 8. Tercer componente: webgraph y centralidad

**Inicio:** después de G-I. **Responsable:** Rust/graph. La corrección del planner ya pertenece a C19; aquí se mejora la producción del grafo y sus señales.

| ID / prioridad | Trabajo | Dependencia | Aceptación y mejora de recursos |
|---|---|---|---|
| W01 · P0 | [ ] Medir extracción de aristas, ordenación, dedup, writer, merge y centralidad con corpus/grafo fijos. | G-I | Tiempo/CPU por millón de aristas, RSS, temporales y bytes de grafo publicados; referencias de resultados conocidas. |
| W02 · P1 | [ ] Aplicar presupuesto global a readers/writers/ordenación/merge y finalización; usar spill a disco. | W01, C21 | Entrada 10× mayor no exige RSS 10×; respeta cuota incluso al coincidir tareas. Conservar la limitación de finalización ya existente como parte del presupuesto. |
| W03 · P0 | [ ] Añadir checkpoints, manifiestos de segmentos y reintento idempotente por fuente. | W02 | Caída durante extracción/merge reanuda; WARC corrupto deja error explícito; aristas y conteos no se duplican. |
| W04 · P1 | [ ] Evaluar y adoptar procesamiento incremental de altas/bajas/cambios donde ahorre trabajo. | W03, I06 | Grafo incremental coincide con rebuild de referencia; medir CPU/I/O y punto de cruce. También puede cerrarse conservando rebuild si la comparación demuestra que no conviene incrementalidad, con decisión documentada. |
| W05 · P1 | [ ] Ajustar particiones, iteraciones y precisión de centralidad según error admitido. | W01, W04 | Publicar error/convergencia frente a referencia; meta ≥20% menos CPU para precisión acordada, sin confundir aproximación con resultado exacto. |
| W06 · P0 | [ ] Versionar juntos grafo, centralidades y señales entregadas a planner/indexador. | W03–W05, I06 | Consumidor rechaza mezcla incompatible; rollback conserva generación anterior; cambio de scores medido en cobertura y ranking. |

**G-W:** grafo/centralidades reanudables, compatibles y dentro del presupuesto, con comparación incremental/rebuild. La salida puede alimentar al crawler cerrado sin modificar sus garantías.

## 9. Cuarto componente: búsqueda distribuida y API

**Inicio:** después de G-W. **Responsable:** Rust/search + operación.

| ID / prioridad | Trabajo | Dependencia | Aceptación y mejora de recursos |
|---|---|---|---|
| S01 · P0 | [ ] Definir corpus de queries, carga objetivo, p50/p95/p99, errores y calidad mínima por endpoint. | G-W | Baseline frío/caliente con tamaño de índice, shards y réplicas; SLO numérico acordado antes de optimizar. |
| S02 · P0 | [ ] Propagar deadline/cancelación y acotar colas, fan-out, candidatos, mensajes y retries; aislar/autenticar search y management internos antes de habilitarlos fuera del nodo. | S01 | Consulta cancelada deja de consumir trabajo; shard lento no retiene indefinidamente; sobrecarga se rechaza antes de OOM; clientes no autorizados no acceden a servicios internos. Aprovechar límites/timeouts existentes. |
| S03 · P1 | [ ] Perfilar ranking, top-k, serialización, mmap y lecturas; cachear solo con límite y clave de generación. | S01, S02 | Meta ≥20% menos p95 o CPU/consulta en cuello medido; misma calidad acordada; cache miss y RSS publicados, sin resultados de generación anterior. |
| S04 · P0 | [ ] Implementar readiness por generación/shard y promoción/rollback atómicos con calentamiento. | I07, W06, S02 | No publicar mezcla parcial; fallo durante promoción mantiene generación válida; cambio/reversión cumplen SLO en carga. |
| S05 · P0 | [ ] Probar réplica lenta/caída, circuit breakers, límites homogéneos y recuperación sin tormenta. | S02, S04 | Error/latencia bajo fallo comparados con SLO; reintentos no multiplican carga sin presupuesto; estado degradado visible. |
| S06 · P1 | [ ] Revisar compresión/payload/API, agrupación de llamadas, conexiones y cachés públicas/privadas. | S03–S05 | Bytes y CPU/consulta medidos; ningún dato privado entra en caché compartida; reducción de payload conserva contrato de cliente. |

**G-S:** carga objetivo, p99/error/quality gates, sobrecarga y caída de réplica verificados. Readiness, promoción y rollback funcionan sobre las generaciones producidas por los componentes previos.

## 10. Quinto componente: centro operativo integral

**Inicio general:** después de G-S. C24/C27 ya entregan la visibilidad y operación necesarias del crawler. Se amplía el dashboard existente sin exigir una plataforma completa para terminar el primer nodo.

| ID / prioridad | Trabajo | Dependencia | Aceptación |
|---|---|---|---|
| O01 · P0 | [ ] Versionar identidad, heartbeat, build/config, estados, eventos y reason codes para todos los componentes. | G-S, C24 | Offline/stale visibles; compatibilidad N/N-1; un snapshot reconstruible no sustituye estado durable. |
| O02 · P0 | [ ] Proyectar catálogo URL→captura→WARC→decisión→generación con outbox e ingestión idempotente. | O01, C10, I06 | Eventos repetidos/desordenados no falsean conteos; lookup paginado y retención acotada; reconstrucción desde fuentes probada. |
| O03 · P1 | [ ] Añadir vistas de frontier, DLQ, almacenamiento, consumidores, generaciones, recursos y costes. | O02 | Overview/lookup caliente objetivo p95 <2 s; frescura p95 <15 s en carga definida; un operador localiza un cuello en menos de dos minutos. |
| O04 · P0 | [ ] Implementar comandos con actor, motivo, target, revisión, TTL, idempotency key y ACK: pause/resume/drain/retry. | O01–O03 | Repetir o interrumpir comando no duplica efectos; estado solicitado/aplicado separados; se conserva auditoría append-only. |
| O05 · P0 | [ ] Añadir permisos por rol, sesiones, autenticación de servicio y controles de operaciones sensibles. | O04 | Pruebas negativas por rol; viewer no muta, operador no publica índices; secretos y URLs sensibles redactados; cambios destructivos muestran alcance y requieren confirmación. |
| O06 · P1 | [ ] Integrar alertas, runbooks, schedules y webhooks firmados con reintentos acotados. | O02, O04 | Una incidencia no produce una tormenta; schedules sobreviven reinicio; webhooks duplicados conservan identidad. |
| O07 · P0 | [ ] Probar backup/restore y caída del plano de control; dimensionar catálogo/histórico antes de añadir nueva infraestructura. | O02–O06 | Crawl y serving continúan dentro de políticas existentes; catálogo y auditoría restaurados; RPO/RTO declarados y medidos. |

Los roles iniciales son viewer, operador de crawl, publicador de índices y administrador/auditor. Métricas agregadas admiten dimensiones acotadas; URL, dominio, query y UUID pertenecen al catálogo/logs con acceso y retención, no a labels ilimitados.

La elección de una base de control, caché externa o bus se documenta con volumen, latencia, recuperación y coste. No se añade PostgreSQL/Redis/Kafka únicamente por aparecer en una arquitectura objetivo.

**G-O:** trazabilidad URL→generación, órdenes idempotentes, auditoría y restore probados; la caída de la UI no altera el plano de datos.

## 11. Sexto componente: frontend, calidad y cartera de funciones

**Inicio:** después de G-O, salvo mantenimiento imprescindible del dashboard en C24. Responsable: frontend/producto y calidad de búsqueda.

| ID / prioridad | Trabajo | Dependencia | Aceptación |
|---|---|---|---|
| F01 · P1 | [ ] Fijar runtime/gestor/lockfiles, actualizar familias compatibles y separar CI frontend, dashboard, WASM y docs. | G-O | Instalación limpia sin cambiar lockfile; check/test/build; versiones elegidas con soporte comprobado al ejecutar la tarea. |
| F02 · P1 | [ ] Medir carga, interacción, bundle, peticiones y memoria en dispositivos/red definidos; optimizar rutas y assets dominantes. | F01 | Presupuesto numérico antes de cambiar; comparación antes/después de bytes, tiempos e interacción; navegación/SSR/hidratación correctas. |
| F03 · P1 | [ ] Integrar corpus de ranking, ranking-diff y anotación; revisar accesibilidad y estados de error/carga. | S01, F01 | Calidad de búsqueda y tareas de usuario sin regresión; resultados reproducibles; teclado y tamaños móvil/escritorio verificados. |
| F04 · P2 | [ ] Evaluar una función por piloto: intake verificado de cambios, portal webmaster o cobertura/frescura explicable. | F03, C18, O02 | Dominio/solicitud verificados, cuotas y procedencia; se mide beneficio antes de ampliar. Push no indexa contenido sin verificar. |
| F05 · P2 | [ ] Añadir export estructurado, historial/diff o extracción semántica solo donde reduzca reproceso o aporte cobertura útil. | C09, I06, O02 | Cada salida apunta a captura/versión; costes de CPU y retención publicados; límites del extractor probados. |
| F06 · P2 | [ ] Definir retirada/corrección autorizada con tombstones y propagación a índice/cachés/derivados. | I06, S04, O05 | Recorrido end-to-end auditado, plazo medido y retención de fuentes tratada explícitamente; no se modifica solo la UI. |

Se conservan como hipótesis posteriores: planificación híbrida con corpus externos, descubrimiento guiado por demanda agregada, plugins estrechos, PDF/JS selectivo y extracción asistida offline. Cada piloto debe demostrar calidad, frescura o ahorro frente al coste; no bloquea G-C ni fuerza una reescritura del motor. Las comparaciones de mercado y versiones concretas de agosto requieren nueva comprobación cuando se utilicen para decidir.

**G-F:** F01–F03 aceptados, instalación/build reproducibles y presupuestos de carga, interacción, accesibilidad y calidad medidos. F04–F06 son pilotos condicionados a una necesidad definida; su activación o aplazamiento se documenta y no obliga a implementar toda la cartera para cerrar esta fase.

## 12. Actualizaciones y ahorro en compilación y operación

Los habilitadores del crawler se ejecutan bajo C22; el resto acompaña al componente correspondiente. No crear un segundo frente de migración general.

| Área | Tarea de mantenimiento | Momento | Evidencia de cierre |
|---|---|---|---|
| Builder Rust/C/C++ | Fijar imagen y herramientas, declarar MSRV probado, resolver/aislar dependencias nativas, verificar submódulos y formato. | C22; después por componente. | Build desde entorno limpio, versiones registradas y checks pertinentes. Elegir versiones soportadas al implementar, sin reutilizar automáticamente las recomendadas en agosto. |
| Release | Separar símbolos, medir ThinLTO/linker y eliminar dependencias/features innecesarias. | C22 e I04. | Tamaño/tiempo de build y arranque comparados; diagnóstico por build ID conservado. |
| Serialización/estado | Inventariar Bincode Git y formatos persistidos; envelope/versiones y fixtures antes de actualizar. | C09/C10/C25, I06. | Lectura N/N-1, migración y rollback; no cambiar formato accidentalmente con una dependencia. |
| HTTP/TLS/S3 | Actualizar por familia, manteniendo bancos C04/C05/C08. | Después de consolidar su baseline o por fallo concreto. | Políticas de red, pooling, cuerpos, timeout, checksum y recuperación sin regresión. |
| CI | Usar instalaciones bloqueadas, cachés por lockfile/toolchain, jobs selectivos y una sola ejecución de cada suite. | C22/F01. | Medir minutos de runner, CPU y tamaño de caché; no ahorrar saltando contratos necesarios. |
| Cadena de suministro | Inventario/SBOM, revisión de dependencias y procedencia de artefactos. | Release de cada componente. | Dependencias y build identificables; excepciones tienen motivo y revisión. |
| Python/PyO3/WASM | Fijar entornos de herramientas y probar compatibilidad por paquete. | Cuando su componente entre en turno. | Scripts/ruedas/WASM reproducibles; no convertirlos en dependencia innecesaria del motor de rastreo. |
| Despliegue | Separar binario crawler, runner y dashboard; instalar solo lo necesario para cada rol. | C22/C27. | El motor puede funcionar sin UI; Node pertenece al servidor del dashboard si se habilita y Python al runner utilizado. |

## 13. Protocolo de medición y aceptación de rendimiento

Cada optimización empieza por un cuello observado y termina con un informe comparable. Los porcentajes objetivo de este plan son **metas de trabajo**, no mejoras ya obtenidas ni compromisos acumulables.

1. Guardar baseline y candidato con mismo corpus, configuración funcional, hardware y cuotas. Publicar commit, cambios locales, binario, lockfile y hashes de datos.
2. Separar warmup de medición; ejecutar al menos tres repeticiones y conservar mediana, MAD, mínimo/máximo y muestras. Usar comparaciones emparejadas cuando corresponda.
3. Medir CPU de proceso/etapa, tiempo de pared, RSS/picos, FD, I/O, red, colas, fsync, espacio temporal y documentos útiles. Asignaciones acumuladas no son RSS.
4. Reconciliar cobertura, exclusiones, errores, bytes y contenido; una mejora que omite trabajo válido no supera aceptación.
5. Probar límites con cuerpos grandes, encodings, redirecciones, DNS adversarial, sitemap cíclico, sitios lentos, 429/503, S3/consumidor caído y reinicios. Mantener carga sintética fuera del estado operativo.
6. En runner estable, una regresión mediana >5% en escenario crítico requiere investigación y decisión documentada; si hay demasiado ruido, repetir con control antes de concluir.
7. Hacer pilotos reales graduales y respetuosos después de las pruebas locales. La capacidad de disco/CPU local y el rendimiento de Internet se publican por separado.
8. Cuando una meta no se alcance, conservar el resultado y decidir ajuste/retirada con evidencia. No sustituirla por un benchmark de otra etapa.

| Área | Objetivo inicial de operación | Condición que impide ampliar |
|---|---|---|
| Durabilidad | Cero ACK perdidos dentro del perfil declarado; todo pendiente/excluido visible. | Cualquier pérdida, corrupción confirmada o conteo sin reconciliar. |
| Memoria | Como margen inicial, RSS p95 <70% y pico <85% de cuota durante carga representativa. | OOM o presión sostenida; recalibrar antes de subir concurrencia. |
| CPU y colas | CPU-ms por documento útil y espera por etapa estables; capacidad medida. | Más workers aumentan espera/CPU sin aumentar salida útil. |
| Spool/disco | Reserva para operación y recuperación; alerta inicial al 80% y por tiempo estimado hasta llenado. | Consumidor/publicación detenidos sin capacidad suficiente; se frena admisión. |
| Red/cortesía | Cero violaciones de política; 429/timeouts por cohorte comparables con baseline. | Ahorro aparente por menor cobertura o aumento injustificado de presión remota. |
| Frescura | Lag p95 por clase de sitio acordado antes de habilitar recrawl adaptativo. | Menos tráfico a costa de incumplir la frescura acordada. |
| Serving | QPS, p95/p99, errores y calidad definidos en S01. | Promoción sin readiness, smoke queries o rollback. |

## 14. Seguimiento, trazabilidad y próximas tareas

### 14.1 Próximo ciclo de trabajo

El próximo ciclo parte de C01–C10 implementados y de la [verificación local de C09–C10](crawler-c09-c10-verificacion-2026-10-12.json).

1. **C08.1:** consolidar evidencia y corregir estados/documentación; es la primera tarea abierta.
2. **C11.1/C12.1/C12.2/C13.1 completados:** conservar la evidencia de identidad, turnos y robots de [C11–C13](crawler-c11-c13-resultados-2026-10-12.md); siguientes tareas abiertas C14–C16/C20/C25 según dependencias.
3. **C08.3–C08.4:** ampliar mediciones de agrupación/publicación y coste al crecer los segmentos, conservando las regresiones aceptadas en C08.2 y C09–C10.
4. Añadir al mismo flujo los habilitadores C22 que bloqueen build/pruebas y las regresiones C16.1/C20.1; no activar modos todavía sin cierre.

### 14.2 Registro requerido al cerrar una tarea

Cada entrega registra ID, dueño/revisor, estado, commit y cambios locales, prueba funcional, informe de recursos, efecto en cobertura/durabilidad, dependencias y rollback. Las tareas sin evidencia permanecen abiertas. Al final de cada ciclo se actualizan este plan y el estado de detalle del crawler, sin mantener calendarios o porcentajes de avance contradictorios.

### 14.3 Correspondencia con el backlog de agosto

| Referencias anteriores | Ubicación vigente |
|---|---|
| CR-001/003: leases y shutdown | C10, C25 y C27; separar un nodo de clúster. |
| CR-002/007: WARC/spool/streaming | C08–C09/C21 e I02–I03. |
| CR-004: SSRF | C04 implementado; regresiones C13/C27 y políticas de todas las nuevas rutas. |
| CR-005/014, UP-001..004/010 | C22 y mantenimiento de la sección 12. |
| CR-006/008: métricas/pipeline | C03/C05/C07 implementados; completar C23–C24. |
| CR-009..011: dedup/traps/recrawl | C16–C18. |
| CR-012/013, UP-005 | I01–I08, W06 y S04. |
| CR-015, CC-001..020, FM-02/FT-004 | C24/C27 para operación mínima; O01–O07 y S04 para centro integral. |
| UP-006 | C25–C26 y compatibilidad de estado/protocolo. |
| UP-007..009 | F01 y mantenimiento de tooling por componente. |
| FM-01/08, FT-001 | C22–C24: diagnóstico, perfiles y admisión por recursos. |
| DX-01..14, FT-002..010, CR-016..018 | C17–C19, I06, O02, F04–F06 y C28 según capacidad; mantener pilotos fuera del camino crítico. |

La actualización de este plan es documental: no ejecuta benchmarks nuevos, no altera configuración ni despliega componentes. Las fuentes operativas principales son el [plan detallado del crawler](plan-evolucion-crawler-2026-10.md), el [contrato C08](crawler-c08.md), sus [mediciones](crawler-c08-resultados-2026-10-12.md), la [guía local](dashboard-crawler.md), el [README del dashboard](../tools/crawler-dashboard/README.md) y la [guía batch](despliegue-crawler.md). Sus discrepancias se resuelven como parte de C08.1 antes de utilizarlas como prueba de cierre.
