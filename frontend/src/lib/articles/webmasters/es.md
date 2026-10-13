# Rastreador AstianGO

AstianGO es un motor de búsqueda web [de código abierto](https://github.com/goastian/astiango/). AstianGOBot es el nombre de nuestro rastreador que recopila páginas de la web para crear el índice.
Está escrito en Rust y el código fuente se puede encontrar [aquí](https://github.com/goastian/astiango/tree/main/crates/core/src/crawler).
El rastreador utiliza el agente de usuario `Mozilla/5.0 (compatible; AstianGOBot/0.2; open source search engine; +https://astiango.com/webmasters)`.

## cortesía

AstianGOBot es un rastreador educado. Respeta el archivo [robots.txt](https://en.wikipedia.org/wiki/Robots.txt) del sitio web que está rastreando e intenta no sobrecargar el servidor.

### Cálculo del tiempo de espera

El rastreador espera un cierto tiempo entre solicitudes al mismo dominio. El tiempo de espera se calcula mediante:
_min((2^cortesía) \* max(fetchtime, 5 seg), 180 seg)_

Dónde:

- _cortesía_ es el factor de cortesía (comenzando en 2)
- _fetchtime_ es el tiempo que tomó buscar la página anterior

El rastreador esperará al menos 5 segundos entre solicitudes y como máximo 180 segundos.

### 429 Manejo de respuestas

Si el rastreador recibe una respuesta 429 (Demasiadas solicitudes) del servidor:

1. Se aumenta el factor de cortesía, duplicando el tiempo entre cada solicitud.
2. Este factor de mayor cortesía persiste mientras dure el rastreo de ese dominio específico.
3. El rastreador nunca disminuirá el factor de cortesía de un dominio que haya devuelto una respuesta 429.

Si el rastreador no ha recibido ninguna respuesta 429 de un servidor, puede disminuir gradualmente el factor de cortesía con el tiempo, pero nunca por debajo de 0.

### Robots.txt

El rastreador busca el token AstianGOBot en el archivo [robots.txt](https://www.robotstxt.org/about.html) para determinar qué páginas (si las hay) puede rastrear.

Si desea restringir el acceso a parte de su sitio, agregue lo siguiente a su archivo robots.txt

```
User-agent: AstianGOBot
Disallow: /private
```

Esto asegurará que AstianGOBot no acceda a ninguna página con el prefijo '/privado'.
También puede restringir el acceso de AstianGOBot a cualquier página de su sitio.

```
User-agent: AstianGOBot
Disallow: /
```

El archivo robots.txt se almacena en caché durante 1 hora, por lo que los cambios en el archivo deben respetarse con bastante rapidez.

## Contáctanos

Si tiene alguna inquietud o mala experiencia con nuestro rastreador, no dude en comunicarse con nosotros en [contact@astian.org](mailto:contact@astian.org). Lo más probable es que otros experimenten los mismos problemas y nos encantaría solucionarlos.