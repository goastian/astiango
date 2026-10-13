# AstianGO Crawler

AstianGO ist eine [Open Source](https://github.com/goastian/astiango/) Websuchmaschine. AstianGOBot ist der Name unseres Crawlers, der Seiten aus dem Web sammelt, um den Index zu erstellen.
Es ist in Rust geschrieben und der Quellcode kann [hier](https://github.com/goastian/astiango/tree/main/crates/core/src/crawler) gefunden werden.
Der Crawler verwendet den Benutzeragenten `Mozilla/5.0 (compatible; AstianGOBot/0.2; open source search engine; +https://astiango.com/webmasters)`.

## Höflichkeit

AstianGOBot ist ein höflicher Crawler. Es respektiert die Datei [robots.txt](https://en.wikipedia.org/wiki/Robots.txt) der Website, die es crawlt, und versucht, den Server nicht zu überlasten.

### Wartezeitberechnung

Der Crawler wartet eine bestimmte Zeit zwischen Anfragen an dieselbe Domain. Die Wartezeit errechnet sich aus:
_min((2^höflichkeit) \* max(fetchtime, 5 sec), 180 sec)_

Wo:

- _höflichkeit_ ist der Höflichkeitsfaktor (ab 2)
- _fetchtime_ ist die Zeit, die zum Abrufen der vorherigen Seite benötigt wurde

Der Crawler wartet zwischen den Anfragen mindestens 5 Sekunden und höchstens 180 Sekunden.

### 429 Antwortverarbeitung

Wenn der Crawler eine 429-Antwort (Too Many Requests) vom Server erhält:

1. Der Höflichkeitsfaktor wird erhöht und die Zeit zwischen den einzelnen Anfragen verdoppelt.
2. Dieser erhöhte Höflichkeitsfaktor bleibt für die Dauer des Crawls für diese bestimmte Domain bestehen.
3. Der Crawler verringert niemals den Höflichkeitsfaktor für eine Domain, die eine 429-Antwort zurückgegeben hat.

Wenn der Crawler keine 429-Antworten von einem Server erhalten hat, kann er den Höflichkeitsfaktor im Laufe der Zeit schrittweise verringern, jedoch niemals unter 0.

### Robots.txt

Der Crawler sucht in der Datei [robots.txt](https://www.robotstxt.org/about.html) nach dem Token AstianGOBot, um zu bestimmen, welche Seiten (falls vorhanden) er crawlen darf.

Wenn Sie den Zugriff auf einen Teil Ihrer Website einschränken möchten, fügen Sie Folgendes zu Ihrer robots.txt-Datei hinzu

```
User-agent: AstianGOBot
Disallow: /private
```

Dadurch wird sichergestellt, dass AstianGOBot nicht auf Seiten mit dem Präfix „/private“ zugreift.
Sie können AstianGOBot auch daran hindern, auf jede Seite Ihrer Website zuzugreifen

```
User-agent: AstianGOBot
Disallow: /
```

Die robots.txt-Datei wird eine Stunde lang zwischengespeichert, sodass Änderungen an der Datei relativ schnell berücksichtigt werden sollten.

## Kontaktieren Sie uns

Wenn Sie Bedenken oder schlechte Erfahrungen mit unserem Crawler haben, zögern Sie bitte nicht, uns unter [contact@astian.org](mailto:contact@astian.org) zu kontaktieren. Die Chancen stehen gut, dass andere die gleichen Probleme haben und wir sie gerne beheben würden.