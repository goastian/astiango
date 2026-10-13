# AstianGO Crawler

AstianGO è un motore di ricerca web [open source](https://github.com/goastian/astiango/). AstianGOBot è il nome del nostro crawler che raccoglie le pagine dal web per costruire l'indice.
È scritto in Rust e il codice sorgente può essere trovato [qui](https://github.com/goastian/astiango/tree/main/crates/core/src/crawler).
Il crawler utilizza l'agente utente `Mozilla/5.0 (compatible; AstianGOBot/0.2; open source search engine; +https://astiango.com/webmasters)`.

## Cortesia

AstianGOBot è un crawler educato. Rispetta il file [robots.txt](https://en.wikipedia.org/wiki/Robots.txt) del sito web che sta scansionando e cerca di non sovraccaricare il server.

### Calcolo del tempo di attesa

Il crawler attende un certo periodo di tempo tra le richieste allo stesso dominio. Il tempo di attesa è calcolato da:
_min((2^gentilezza) \* max(fetchtime, 5 sec), 180 sec)_

Dove:

- _gentileness_ è il fattore cortesia (a partire da 2)
- _fetchtime_ è il tempo impiegato per recuperare la pagina precedente

Il crawler attenderà almeno 5 secondi tra le richieste e un massimo di 180 secondi.

### 429 Gestione delle risposte

Se il crawler riceve una risposta 429 (Too Many Requests) dal server:

1. Il fattore cortesia viene aumentato, raddoppiando il tempo tra ogni richiesta.
2. Questo maggiore fattore di cortesia persiste per tutta la durata della scansione per quello specifico dominio.
3. Il crawler non diminuirà mai il fattore cortesia per un dominio che ha restituito una risposta 429.

Se il crawler non ha ricevuto alcuna risposta 429 da un server, potrebbe ridurre gradualmente il fattore cortesia nel tempo, ma mai al di sotto dello 0.

### Robot.txt

Il crawler cerca il token AstianGOBot nel file [robots.txt](https://www.robotstxt.org/about.html) per determinare quali pagine (se presenti) è autorizzata a scansionare.

Se desideri limitare l'accesso a una parte del tuo sito, aggiungi quanto segue al file robots.txt

```
User-agent: AstianGOBot
Disallow: /private
```

Ciò garantirà che AstianGOBot non acceda ad alcuna pagina con il prefisso "/private".
Puoi anche impedire ad AstianGOBot di accedere a qualsiasi pagina del tuo sito

```
User-agent: AstianGOBot
Disallow: /
```

Il file robots.txt viene memorizzato nella cache per 1 ora, quindi le modifiche al file dovrebbero essere rispettate abbastanza rapidamente.

## Contattaci

In caso di dubbi o esperienze negative con il nostro crawler, non esitare a contattarci all'indirizzo [contact@astian.org](mailto:contact@astian.org). È probabile che altri riscontrano gli stessi problemi e ci piacerebbe risolverli.