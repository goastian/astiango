# Plan de evolución del frontend público de AstianGO

Fecha: **15 de octubre de 2026 (Bogotá)**. Código revisado hasta `35fa98d1` (`include build process`). Componente: **web pública del buscador**, especialmente home y resultados. [Plan maestro](plan-maestro-mejora-2026-08.md#11-frontend-calidad-y-cartera-de-funciones) · [estado de componentes](checklist-componentes-2026-10-15.md) · [referencias visuales y capturas](frontend-referencias-visuales-2026-10-15.md).

**Estado: frontend funcional desplegado en local con Vite HMR [x]; aceptación ampliada de FE01/FE10/FE12 y exposición pública pendientes [ ].** [Entrega e integración local](frontend-publico-resultados-2026-10-15.md). La planificación inicial precede a esta implementación; las tareas FE desarrollan F01–F03 del maestro y no cierran F04–F06.

## 1. Resultado buscado y límites

Que una consulta muestre resultados reales del índice propio, con títulos, destinos, resúmenes, filtros y navegación coherentes. El usuario debe reconocer qué está buscando, qué ocurrió y qué puede hacer a continuación. Mejorar apariencia incluye corregir fallos de contrato, estado y concurrencia; no basta cambiar CSS.

Crawler, indexador, webgraph y Search/API permanecen activos durante el desarrollo. El frontend se construye/despliega por separado, sobre generaciones compatibles; no se vacían datos para corregir una pantalla. Las referencias de Brave, Qwant, DuckDuckGo y Bing orientan jerarquía y legibilidad; marca, componentes y capacidades siguen siendo de AstianGO.

La primera entrega es búsqueda web fiable. Imágenes, vídeos, noticias, discussions, widgets, IA, voz o cámara solo se presentan como controles operativos si existe un backend compatible. La API comercial S07 sigue solo planificada. El frontend no cambia el ranking ni elimina resultados del motor para aparentar calidad; un defecto del índice/ranking se asigna a ese componente.

Los correctivos P0 de resultados pueden avanzar con Search/API local actual, sin esperar a completar todo O01–O07 o una certificación sostenida de producción. La exposición pública conserva G-S/O05/G-F y una decisión de despliegue explícita.

## 2. Preparación realizada

- [x] Inventario inicial de home, carga de búsqueda, API, SERP, resultado, snippets, búsqueda/autosuggest, SSR e i18n.
- [x] Visita visual a home/búsqueda de cuatro buscadores; capturas y limitaciones registradas.
- [x] Revisión de una ilustración de SERP publicada en la ayuda oficial de Brave, diferenciada de captura live.
- [x] Correspondencia con F01–F03 y backlog separado por entregas/aceptación.
- [x] Reproducir y corregir el flujo operativo contra Search/API local: resultados, errores, navegación, móvil e idiomas; evidencia en la entrega local. El corpus ampliado sigue en FE01.

La revisión visual externa no demostró todas las SERP: Brave y DuckDuckGo presentaron CAPTCHA y Qwant un error de servicio. Bing sí entregó una lista orgánica. El [documento visual](frontend-referencias-visuales-2026-10-15.md) distingue páginas observadas, material oficial y partes no verificadas. No se sortearon los bloqueos.

## 3. Hallazgos iniciales y propietarios

Son hallazgos de lectura de código, no una afirmación de haber reproducido todos los fallos en producción. FE01 conserva caso, ruta, payload, captura y versión antes de cerrar un defecto.

| Hallazgo | Fuente actual | Impacto / prioridad | Entrega |
|---|---|---|---|
| El cliente usa `POST /beta/api/search`; la API local nueva es `GET /v1/web/search`, con otros campos. | [API frontend](../frontend/src/lib/api/index.ts), [SearchParams](../frontend/src/lib/search.ts), [API local](../scripts/search-api.py) | Integración pendiente de verificar; no basta cambiar la URL. P0. | FE02 |
| La respuesta pública espera `webpages`, `numHits`, `hasMoreResults` y snippets por fragmentos; la local entrega `results`, `generation` y snippet de texto. | [Tipos](../frontend/src/lib/api/index.ts), [SERP](../frontend/src/routes/search/Serp.svelte) | Riesgo de campos ausentes, contadores/paginación incorrectos y errores de render. P0. | FE02/FE05 |
| `search()` agrupa resultados principales, widget, sidebar, discussions y spellcheck en `Promise.all`. | [search.ts](../frontend/src/lib/search.ts) | Un auxiliar lento/fallido puede retrasar/rechazar toda la búsqueda. P0. | FE03/FE09 |
| El `catch` de la búsqueda muestra artículos promocionales etiquetados como preview. | [Página de resultados](../frontend/src/routes/search/+page.svelte) | La pantalla no explica el fallo de esa consulta; el preview no responde a ella. P0. | FE03 |
| Carga cliente reactiva sin cancelación/identidad de búsqueda principal; existe camino SSR adicional. | [Página](../frontend/src/routes/search/+page.svelte), [load](../frontend/src/routes/search/+page.ts) | Revisar respuestas fuera de orden y peticiones dobles; todavía no se afirma reproducción universal. P0. | FE04 |
| Pestañas cambian `type`, pero `extractSearchParams` no lo interpreta y All tiene clase activa fija. | [Página](../frontend/src/routes/search/+page.svelte), [parser](../frontend/src/lib/search.ts) | La navegación promete otra categoría sin contrato efectivo en esa ruta. P0. | FE08 |
| Títulos en una línea en desktop, `id="snippet-text"` repetido y botones de resultado de 28 px. | [Result](../frontend/src/routes/search/Result.svelte) | Recorte/identificadores no únicos y revisión de interacción táctil. P1/P0 accesibilidad. | FE05/FE07/FE10 |
| Autosuggest se solicita al cambiar query sin debounce explícito; cancela el anterior, pero no comprueba secuencia de respuesta. | [Searchbar](../frontend/src/lib/components/Searchbar.svelte) | Revisar tráfico evitable, selección vacía y respuestas antiguas. P1. | FE04/FE06 |
| Fallback del API base a `http://0.0.0.0:3000`; CSP/connect y CSRF necesitan revisión del perfil público. | [layout](../frontend/src/routes/+layout.ts), [config](../frontend/svelte.config.js) | Dirección de escucha no equivale a endpoint público; revisar origen/secretos y rutas autorizadas. P0 antes de exposición. | FE02/FE14 |
| El shell y componentes usan nombres `qwant-*` y columnas reservadas a módulos opcionales. | [Página](../frontend/src/routes/search/+page.svelte), [Serp](../frontend/src/routes/search/Serp.svelte), [tokens](../frontend/tokens.css) | Definir componentes/tokens propios y distribución dependiente de contenido real. P1. | FE07 |

**Prioridad heurística preliminar: 4/10, documental y sin validación de la web real en navegador.** Se descuentan tres filas mayores: navegación/categoría engañosa, feedback de carga insuficiente y recuperación de error sustituida por preview. La ausencia de inspección funcional completa no se puntúa como fallo demostrado. Para acercarse a 10/10: cerrar esas tres causas en FE03/FE08, completar teclado/foco FE10 y volver a evaluar tareas reales con FE14. No es una métrica de rendimiento ni una puntuación de usuarios.

### Cómo asignar un error de resultados

| Observación | Propietario principal | Comprobación |
|---|---|---|
| La respuesta API es correcta pero la tarjeta falta, cambia de orden o muestra otra URL. | Frontend FE02/FE05/FE07 | Comparar respuesta normalizada y DOM de la misma request/generación. |
| La página muestra una query anterior o mezcla idiomas/filtros. | Frontend FE04/FE08/FE11 | Registrar identidad de petición, URL/contexto y orden de respuestas. |
| La API rechaza capacidades que la UI presenta como activas. | Contrato FE02/FE08 + Search | Capability matrix y decisión explícita de soporte/adaptación. |
| La API responde correctamente con documentos irrelevantes. | Search/ranking/índice, con FE01/F03 | Query/corpus/generación y juicio de relevancia; no reordenar ni ocultar en CSS. |
| No hay nueva captura/generación o hay cuota. | Crawler/ingesta/grafo/operación | Trazar captura→decisión→generación→serving; no confundirlo con error de render. |

## 4. Dirección visual propuesta

Conservar la identidad AstianGO y partir de los tokens actuales: fondo claro, contraste de enlaces, colores de marca y temas existentes. Validar sus valores reales de contraste; comentarios del CSS no acreditan esa validación. La tipografía de resultados prioriza lectura frente a apariencia de titular publicitario.

- **Home:** marca, descripción breve, input dominante de aproximadamente 600–680 px en desktop y anchura disponible en móvil. Búsqueda por Enter/botón y ajustes discretos; nada esencial depende de autosuggest.
- **Cabecera SERP:** marca compacta y barra alineada con la columna de lectura; query editable siempre accesible. Navegación/filtros describen capacidades reales.
- **Resultado:** procedencia/dominio legible, título como enlace principal y resumen; fechas/favicons/sitelinks solo cuando existen. Títulos de hasta dos líneas como punto de partida desktop y adaptación móvil; evitar recorte arbitrario de significado.
- **Layout:** columna de aproximadamente 640–760 px y sidebar opcional de 280–320 px, espacio consistente y contenedor ajustado al viewport. Sin sidebar, conservar ancho legible y retirar el hueco estructural que corresponda.
- **Móvil:** una columna; filtros desplegables y menús con foco correcto. Conservar query, posición y estado al volver de un resultado.
- **Estados:** carga, vacío real, fallo, saturación y capacidad no disponible tienen presentación distinta. Resultados anteriores visibles solo con identificación de que son anteriores.

Estas medidas son valores iniciales para prototipar/contrastar, no mediciones de los competidores ni una maqueta aprobada. [Capturas, decisiones y límites de la referencia](frontend-referencias-visuales-2026-10-15.md).

## 5. Contrato objetivo con Search/API

Se propone una capa de adaptación en el frontend/servidor del frontend, manteniendo compatible la API existente. Antes de elegir sustituir el backend histórico, FE02 inventaría las capacidades que se perderían. Mismo origen hacia el navegador; backend/config privada en el servidor. El navegador no recibe tokens de control ni usa los endpoints administrativos del dashboard.

| Dato | Contrato del frontend propuesto |
|---|---|
| Query/contexto | Texto exacto, página, tamaño, SafeSearch, idioma UI y opciones de contenido claramente separados. |
| Resultado | URL de destino intacta, título, snippet de texto/fragmentos seguros y metadata opcional validada. |
| Total | Exacto/aproximado/desconocido; `documents` del índice no es el total de coincidencias. No fabricar `numHits`. |
| Paginación | `hasMore`/cursor confirmado o estado desconocido explícito. No deducir certeza únicamente de recibir `count` resultados. |
| Estado | Loading, success, empty, error, cancelled; degradación solo de módulos auxiliares diferenciada. |
| Identidad | Request ID/generación/contexto para reconciliar UI/API; su detalle técnico queda en diagnóstico. |
| Capacidades | Web/imágenes/vídeos/noticias/ópticas/región/spellcheck/widgets/autosuggest explícitos. Sin soporte no se simulan con otra query web. |

La nueva ruta local usa `count/page/safe_search/language`; el frontend histórico usa `p/ss/gl/optic` y preferencias adicionales. El mapeo debe ser deliberado y comprobable. `ui_lang` no demuestra filtrado de documentos por idioma; la capacidad `language` local tampoco promete ese filtro. Una región visualmente seleccionada no puede ignorarse silenciosamente si el backend no la admite.

## 6. Backlog específico

### FE01. Inventario, reproducción y baseline del frontend — P0 · M

- [x] Fijar frontend/config/API/generación por hash, runtime/lockfile y rutas home, resultados y ajustes; distinguir perfil local y público.
- [ ] Reproducir defectos con consultas conocidas y payload guardado; corpus de estados/títulos/URLs/idiomas y capturas desktop/móvil, sin inyectarlos en contadores operativos.
- [x] Medir requests por búsqueda, JS/CSS/WASM/fuentes transferidos, render/CPU/memoria navegador y primer resultado; separar latencia del API.
- **Aceptación:** una orden reproduce el caso y conserva resultado esperado/actual, consola/red y captura; cada defecto tiene propietario y severidad. Referencias externas no sustituyen el baseline propio. Tres repeticiones solo si se publican cifras comparativas.
- **Dependencias:** ninguna para inventario; Search/I/W locales para resultados reales. **Riesgo:** atribuir al frontend errores de ranking/cobertura.

### FE02. Adaptador de API, capacidades y configuración efectiva — P0 · M

- [x] Definir y validar el modelo de presentación; adaptar contrato histórico/local sin conversiones por simple cast TypeScript ni campos inventados.
- [x] Fijar API pública mediante configuración estricta y proxy del mismo origen cuando corresponda; secretos y control permanecen del lado servidor.
- [x] Publicar matriz de capacidades y reglas de total/paginación/SafeSearch/idioma/región/ópticas; mantener compatibilidad antes de retirar rutas.
- **Aceptación:** una respuesta real de generación identificada produce las mismas URLs, orden y campos útiles en DOM; errores 400/429/503/504 y versión/esquema incompatible tienen causa explícita. Home/SSR no dependen de `0.0.0.0` como endpoint del cliente.
- **Dependencias:** FE01, S00–S06. **Riesgo:** perder funcionalidades históricas al conectar una API con capacidades menores.

### FE03. Estados fiables, fallo y SSR/hidratación — P0 · M

- [x] Implementar estados de carga, éxito, vacío real, error y cancelación; conservar query y ofrecer recuperación contextual.
- [x] Sustituir el preview promocional en el camino de error; separar cualquier demo de la búsqueda operativa. Aislar fallos auxiliares.
- [x] Elegir política SSR/cliente explícita, también para acceso directo, refresh y sin JavaScript cuando se ofrezca ese soporte.
- **Aceptación:** API caída no genera artículos que aparenten responder la consulta; cero coincidencias no se confunde con error; no hay pantalla en blanco permanente ni doble consulta principal por hidratación. La navegación conserva el contexto.
- **Dependencias:** FE02. **Riesgo:** presentar resultados anteriores como nuevos o convertir un fallo en un vacío.

### FE04. Concurrencia, cancelación y respuestas fuera de orden — P0 · M

- [x] Asociar cada búsqueda a identidad/contexto y cancelar la anterior en query/filtros/navegación; descartar respuestas antiguas aunque el transporte no se interrumpa.
- [x] Propagar deadlines de cliente/servidor sin retries automáticos ilimitados; limpiar listeners/timers al desmontar.
- [ ] Debounce acotado de autosuggest, secuencia de respuesta y selección segura con lista vacía; respetar composición IME.
- **Aceptación:** query A lenta seguida de B rápida muestra solo B; atrás/adelante y cambiar filtro no mezclan resultados; rechazo/abort de un auxiliar no rompe la consulta. Una búsqueda produce una petición principal por navegación efectiva.
- **Dependencias:** FE02/FE03. **Riesgo:** reactividad que duplique trabajo o que cancele también la búsqueda nueva.


**Perfil local:** autosuggest no está disponible en Search/API; se eliminan sus solicitudes. Debounce/secuenciación de sugerencias quedan condicionados a habilitar un backend compatible. IME del formulario sí se respeta.
### FE05. Tarjeta de resultado y snippets seguros — P0 · M

- [x] Normalizar título vacío/largo, URLs/IDN/escapes, snippet ausente, fechas y campos opcionales; preservar identidad y destino C11.
- [x] Mantener texto escapado/fragmentos de highlight; no insertar HTML remoto sin contrato seguro. Unificar salida nativa de texto y fragmentos históricos.
- [x] Corregir IDs repetidos y semántica de enlaces; definir favicon/imagen fallback con espacio estable y sin llamadas arbitrarias a terceros.
- **Aceptación:** lista y enlaces corresponden al payload en el mismo orden; URLs firmadas conservan parámetros/escapes, protocolos no navegables se rechazan, HTML remoto no ejecuta scripts y campos ausentes no tumba el resto. No se fabrican fechas ni se deduplica/rerankea silenciosamente en cliente.
- **Dependencias:** FE02/FE03. **Riesgo:** arreglar apariencia cambiando el contenido o destino real del resultado.

### FE06. Home y búsqueda compartida — P1 · S

- [x] Aplicar dirección visual a home y Searchbar compartida, con marca propia, foco visible, limpiar/buscar y navegación secundaria.
- [x] Conservar preferencias/query y distinguir enviar, elegir sugerencia y editar; búsqueda usable aunque autosuggest no exista.
- [x] Adaptar altura al viewport/teclado móvil; reducir contenido y recursos no necesarios para la tarea principal.
- **Aceptación:** desde home se busca con Enter, botón o sugerencia y se llega a la query correcta; el teclado/zoom no ocultan controles; input tiene nombre accesible y no necesita instrucciones extensas.
- **Dependencias:** FE03/FE04, diseño de FE07. **Riesgo:** una home atractiva que introduzca otra barra o estado incoherente con resultados.

### FE07. Layout SERP, tipografía y densidad — P1 · M

- [x] Convertir dirección visual en componentes/tokens propios: cabecera, columna principal, tarjeta, auxiliares y footer/paginación.
- [x] Corregir ancho, wrapping de títulos/URLs, contraste, espacios y posicionamiento; sidebar solo cuando haya contenido.
- [x] Preparar estados de error/vacío/loading con el mismo layout y verificar temas existentes/reduced motion.
- **Aceptación:** 360/390/768/1440 px sin overflow horizontal; títulos y resúmenes del corpus siguen legibles, enlaces/visitados distinguibles y primer resultado accesible sin bloque vacío injustificado. No se cambia orden ni contenido para llenar la pantalla.
- **Dependencias:** FE05 y referencias visuales; iteración conjunta con FE06. **Riesgo:** exceso de tarjetas/efectos o copiar nombres/dimensiones de otro producto sin adaptación.

### FE08. Navegación, filtros y paginación con semántica real — P0 · M

- [x] Derivar el tab activo de la ruta/capacidad; ofrecer categorías implementadas y explicar/retirar las no disponibles del flujo operativo.
- [x] Conservar query, SafeSearch, idioma y filtros compatibles al paginar/cambiar categoría; validar página y resetearla cuando cambia la consulta.
- [x] Definir backlinks, abrir/nueva pestaña, historial y recuperación de scroll/foco; cifras y siguiente página solo con autoridad del backend.
- **Aceptación:** All no permanece activo en otra categoría; cambiar tab cambia realmente el tipo admitido; atrás/adelante reconstruye los mismos parámetros. No hay páginas negativas, siguientes ficticias ni desaparición silenciosa de filtros.
- **Dependencias:** FE02/FE04. **Riesgo:** confundir una preferencia visual con filtrado o cobertura de una vertical todavía inexistente.

### FE09. Widgets, sidebar, discussions y resultados enriquecidos — P1 · M

- [x] Clasificar módulos necesarios/opcionales y activar cada uno según capacidades; obtener resultados principales independientemente de auxiliares.
- [ ] Cargar auxiliares con presupuesto/deadline propios, sin multiplicar búsquedas principales ni bloquear el primer resultado.
- [ ] Validar relevancia/contexto, límites de contenido y layout estable; ordenar prioridad en móvil.
- **Aceptación:** sidebar/spellcheck/widget lento, vacío o fallido conserva la lista web utilizable; las capacidades no soportadas no generan cinco errores por query. Un módulo tardío pertenece a la consulta vigente y no provoca salto de layout fuera de presupuesto.
- **Dependencias:** FE02–FE04/FE07. **Riesgo:** presentar como soportadas funciones que la API local declara ausentes.


**Perfil local:** los auxiliares no soportados no se solicitan ni se presentan como activos. Los presupuestos/carga tardía se implementan cuando se habilite cada capacidad; no se inventan widgets para cerrar las casillas.
### FE10. Accesibilidad y uso móvil/teclado — P0 · M

- [x] Revisar landmarks, nombre del input, combobox/listbox/options, active descendant, live status y encabezados; IDs únicos.
- [x] Foco/restauración, Escape, Tab y atajos sin capturar escritura/IME en otros inputs; menú de resultado utilizable sin hover y sin trampa de foco.
- [x] Verificar zoom/reflow, contraste, táctil, reduced motion y temas, con objetivo WCAG 2.2 AA para el flujo.
- **Aceptación:** búsqueda→resultado→volver→paginación→error/reintento completa por teclado y móvil; el lector de pantalla identifica estados/resultados. WCAG 2.2 AA de tamaño objetivo mínimo y excepciones revisadas; adoptar 44 px como objetivo de diseño para botones aislados, sin fingir que todo elemento inline exige ese tamaño.
- **Dependencias:** FE05–FE08, desde el primer componente. **Riesgo:** puntuación automática sin revisar interacción real.


**Aceptación pendiente:** revisión con lector de pantalla y corpus ampliado de zoom/foco. La integración actual cubre teclado, menú móvil, landmarks, contraste medido y anchos representativos; no certifica WCAG completo.
### FE11. Siete idiomas y separación de contextos — P1 · M

- [x] Mantener inglés, alemán, portugués, español, italiano, francés y mandarín en home, errores, filtros y resultados/fechas/cifras de la interfaz.
- [x] Separar idioma UI, texto de consulta, región e idioma de documentos según las capacidades reales; conservar parámetros firmados y caracteres.
- [x] Verificar expansión de texto, cobertura CJK, IME y estado activo al cambiar idioma; no traducir ni reescribir automáticamente el documento indexado.
- **Aceptación:** mismo destino/query al cambiar idioma UI; sin claves crudas ni pérdida de caracteres. Formularios, SSR y navegación conservan `ui_lang`; opciones de contenido tienen efecto confirmado o indicación explícita de ausencia.
- **Dependencias:** FE02/FE04/FE08; catálogos existentes. **Riesgo:** confundir interfaz traducida con relevancia/filtrado multilingüe ya demostrados.

### FE12. Rendimiento del navegador y coste de red — P1 · M

- [x] Perfilar JS/CSS/WASM, fuentes, SSR/hidratación, render/DOM e interacciones; descargar solo módulos necesarios por ruta/capacidad.
- [x] Reutilizar la caché/compresión ya existentes sin mezclar generación o contexto; priorizar el resultado principal y evitar preload indiscriminado.
- [x] Medir con el mismo perfil de dispositivo/red/corpus tres repeticiones por comparación; CPU/render y payload separados del tiempo del API.
- **Aceptación:** presupuesto FE01 fijado antes de optimizar; candidato sin regresión funcional ni subida injustificada >5% en escenario crítico. Objetivo orientativo ≥20% menos JS inicial o CPU en el cuello medido, solo si el baseline lo justifica. CWV de campo y cifras de laboratorio se publican separados.
- **Dependencias:** FE01/FE03/FE07/FE09. **Riesgo:** sacrificar contenido o llamar ganancia de frontend a un hit de caché del API.


**Medición:** baseline de producción en tres repeticiones, separado de Vite y de red/CPU nativa. Falta una comparación funcional equivalente para atribuir ≥20% y una muestra de memoria prolongada/CWV de campo; no hay porcentaje global demostrado.
### FE13. Diagnóstico y sección del frontend en el centro de control — P1 · M

- [x] Proyectar build/config/capacidades del frontend, readiness del flujo y estado de la API, sin hacer depender la web pública del dashboard.
- [x] Añadir vistas funcionales Web frontend: Overview, User experience y Diagnostics; errores agregados, requests por búsqueda, latencia API→primer resultado y assets.
- [x] Separar probes/fixtures/usuarios; métricas acotadas sin queries, IPs o URLs completas como etiquetas/logs de uso por defecto.
- **Aceptación:** operador distingue HTML servido con búsquedas rotas, API caída, JS fallido y ausencia real de resultados; un probe autorizado realiza una query y verifica DOM/generación. UI operativa caída no afecta la web ni ingesta. Un HTTP 200 de home no implica readiness de búsqueda.
- **Dependencias:** FE02/FE03; observación mínima desde FE01, coordinación O01/O03/O05. **Riesgo:** crear otro dashboard o publicar telemetría sensible/alta cardinalidad.

### FE14. Integración, despliegue gradual y rollback — P0 · M

- [x] Un runner de integración del flujo con generación real fijada y fallos de API/auxiliares aislados, manteniendo ingesta activa.
- [x] Check/build y CI selectiva reproducibles; preview/canary del frontend separado, readers/contratos compatibles y rollback de assets/config/backend explícito.
- [ ] Verificar home→query→enlace→volver→filtro/página, versiones/noindex y errores; medir coexistencia breve sin parar componentes para probar uno.
- **Aceptación:** cero defectos P0 abiertos en el corpus acordado, contrato y versión identificados, siete idiomas/móvil/teclado y recuperación de fallo; una orden conserva evidencia. Rollback vuelve a frontend/API compatibles sin borrar generaciones ni datos. Publicación externa exige S/O05 y decisión explícita; el preview local no es despliegue público.
- **Dependencias:** primer gate pequeño tras FE02–FE05/FE08; cierre completo tras FE06–FE13. **Riesgo:** sustituir simultáneamente frontend/API/índice y no poder atribuir un fallo.

## 7. Orden de entregas

| Entrega | Tareas | Resultado concreto | Gate |
|---|---|---|---|
| M-F0: búsqueda fiable | FE01–FE05 + FE08 + FE14 mínimo | Query real, tarjeta correcta, error explícito y navegación coherente. | API payload↔DOM reconciliado; auxiliares no bloquean; respuesta antigua descartada. |
| M-F1: lectura y diseño | FE06/FE07 + FE10/FE11 continuos | Home/SERP con identidad propia, móvil y teclado; siete idiomas. | Corpus visual y tareas de usuario sin pérdidas de contenido. |
| M-F2: módulos y recursos | FE09/FE12 + FE13 | Auxiliares opcionales, payload/render medidos y diagnóstico. | Presupuestos y ausencia de dependencia del dashboard. |
| M-F3: cierre de la web | FE14 completo + G-F | Integración, canary/rollback y expediente de aceptación. | Perfil público coordinado con G-S/O05 cuando se habilite. |

FE08 es P0 aunque se enumere después del diseño; se entrega en M-F0. Accesibilidad, idiomas y observación acompañan cada cambio. No esperar al rediseño total para corregir fallos graves de resultados.

## 8. Integración representativa y presupuestos

### Corpus de aceptación

- Query conocida con resultados de una generación real; query vacía, rara y sin coincidencias; parser de frase/filtro admitido y entrada inválida.
- Título/URL muy largos, título vacío, snippet ausente, IDN, parámetros firmados, caracteres de siete idiomas y texto que parece HTML.
- API principal lenta/caída/429/503/504, JSON incompatible y auxiliar fallido; dos consultas con respuestas A/B invertidas.
- Paginación, filtros y atrás/adelante; sin sidebar, con sidebar admitida y módulo tardío; SSR y navegación cliente.
- Sin JS si se ofrece, teclado/IME, zoom/reduced motion, temas y anchos 360/390/768/1440 px.

Una integración pequeña por contrato modificado. Reutilizar fixtures/manifest de Search/índice; los fallos no se inyectan a procesos operativos. No se exige ejecutar todos los unitarios históricos ni miles de validaciones para cada ajuste visual. Las comparaciones de rendimiento sí usan tres repeticiones y mediana/dispersión.

### Criterios iniciales que debe fijar FE01

| Área | Criterio propuesto |
|---|---|
| Exactitud UI | 100% de URLs, orden y contenido esperado del corpus coinciden con la respuesta normalizada; cero preview promocional ante error. |
| Trabajo por navegación | Una query principal por navegación efectiva; auxiliares solo por capacidad y con cotas/deadline; sin retry automático infinito. |
| Tiempo de render | Medir respuesta principal→primer resultado y respuesta→DOM estable; proponer p95 ≤100 ms para una página simple de hasta 20 resultados en el equipo/perfil declarado, revisable con evidencia antes de optimizar. |
| Recursos | Sin crecimiento sostenido tras una secuencia fija de navegación; memoria/requests/DOM por ruta medidos. El API conserva su presupuesto independiente. |
| CWV | Objetivos de referencia de campo p75: LCP ≤2,5 s, INP ≤200 ms, CLS ≤0,1. El laboratorio ayuda al diagnóstico y no certifica p75 de usuarios sin muestra real. |
| Accesibilidad | Flujo WCAG 2.2 AA comprobado, teclado/lector de pantalla/zoom/móvil; automatización acompañada de revisión de tareas. |
| Operación conjunta | Ventana breve de coexistencia con adquisición/consumidores activos; esperar entrada/cortesía con causa es normal. Frontend/dashboard no controlan implícitamente esos procesos. |

Referencias: [Core Web Vitals](https://web.dev/articles/vitals) y [WCAG 2.2](https://www.w3.org/TR/WCAG22/). Los números son objetivos propuestos del plan; no mejoras medidas ni requisitos copiados de la velocidad de competidores.

## 9. Correspondencia con el maestro y estado de cierre

| Maestro | Desarrollo específico | Qué no completa por sí solo |
|---|---|---|
| F01: runtime/lockfiles/CI | FE01/FE12/FE14 | No repara automáticamente todos los scripts históricos ni certifica ejecución remota de CI. |
| F02: carga/interacción/bundle | FE01/FE06/FE07/FE12 | No demuestra un porcentaje global del buscador. |
| F03: relevancia/accesibilidad/i18n | FE02–FE05/FE08–FE11/FE14 | Un ranking defectuoso requiere corrección en Search/índice; no solo CSS. |
| O01/O03/O05 | FE13 y exposición de FE14 | No da por cerrado el centro operativo integral. |
| G-S/S04–S06 | FE02/FE14 | Resultados locales no certifican SLO público ni HA. |
| F04/F05/F06 | Fuera de la primera entrega; contratos de UI se conservan | Portales/exports/retiradas requieren sus aceptaciones originales, no solo pantallas. |
| S07 | Solo planificación comercial vigente | Rediseñar la búsqueda no activa cobros ni credenciales comerciales. |

- [x] Plan y referencias conservados; maestro enlazado y prioridades de correctivos P0 aclaradas.
- [x] M-F0 local: resultados y contrato corregidos/verificados contra el índice activo.
- [x] M-F1 local: home/SERP, móvil, teclado, contraste revisado y siete idiomas; aceptación ampliada de accesibilidad pendiente.
- [x] M-F2 local: auxiliares no disponibles desactivados, baseline/diagnóstico reales y costes parciales explícitos. Ganancias cuantitativas y auxiliares futuros siguen condicionados.
- [ ] M-F3 / G-F: expediente, integración y rollback completados para el perfil declarado.

Cada entrega registra fuentes/build/config/generación, caso esperado/actual, captura, integración, recursos y pendientes por propietario. Las casillas FE solo pasan a [x] con esa evidencia; este documento no marca como arreglados errores que todavía solo han sido identificados.


## 10. Operación de esta entrega

- [x] Frontend Vite/HMR: `http://127.0.0.1:8000/`, servicio independiente `astiango-frontend-local.service`.
- [x] Dashboard: `http://127.0.0.1:3101/#/frontend/overview`; consultas, palabras clave, intención inferida, recursos y diagnósticos.
- [x] Corrección compartida del lector WARC para 304/204/1xx sin cuerpo: no descomprime una representación inexistente; segmento original validado y recuperado en ambos consumidores sin borrar fuentes/cursors.
- [x] Presupuestos del indexador reducidos, manteniendo guard de 512 MiB; entrada detenida por memoria recuperada desde cursor durable.
- [ ] Completar corpus adversarial congelado FE01, lector de pantalla/zoom FE10, memoria prolongada y comparación equivalente FE12, y gate público G-F/G-S/O05. La integración local no sustituye estas puertas.

Las casillas completadas corresponden al perfil web local. No se habilitan cobros, modelos de intención, verticales ni ranking personalizado ausentes. La intención es una regla léxica, no una verdad sobre la persona. El coste monetario requiere tarifas y solo representa CPU nativa/bytes del API; no el coste total del buscador.
