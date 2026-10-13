# Robot d'exploration AstianGO

AstianGO est un moteur de recherche Web [open source] (https://github.com/goastian/astiango/). AstianGOBot est le nom de notre robot qui collecte les pages du Web afin de construire l'index.
Il est écrit en Rust et le code source peut être trouvé [ici](https://github.com/goastian/astiango/tree/main/crates/core/src/crawler).
Le robot d'exploration utilise l'agent utilisateur `Mozilla/5.0 (compatible; AstianGOBot/0.2; open source search engine; +https://astiango.com/webmasters)`.

## Politesse

AstianGOBot est un robot d'exploration poli. Il respecte le fichier [robots.txt](https://en.wikipedia.org/wiki/Robots.txt) du site Web qu'il explore et essaie de ne pas surcharger le serveur.

### Calcul du temps d'attente

Le robot attend un certain temps entre les requêtes adressées au même domaine. Le temps d'attente est calculé par :
_min((2^politesse) \* max(temps de récupération, 5 sec), 180 sec)_

Où:

- _politesse_ est le facteur de politesse (à partir de 2)
- _fetchtime_ est le temps qu'il a fallu pour récupérer la page précédente

Le robot attendra au moins 5 secondes entre les requêtes et au maximum 180 secondes.

### 429 Gestion des réponses

Si le robot reçoit une réponse 429 (trop de requêtes) du serveur :

1. Le facteur de politesse est augmenté, doublant le délai entre chaque demande.
2. Ce facteur de politesse accru persiste pendant toute la durée de l'exploration de ce domaine spécifique.
3. Le robot d'exploration ne diminuera jamais le facteur de politesse pour un domaine qui a renvoyé une réponse 429.

Si le robot n'a reçu aucune réponse 429 d'un serveur, il peut diminuer progressivement le facteur de politesse au fil du temps, mais jamais en dessous de 0.

### Robots.txt

Le robot recherche le jeton AstianGOBot dans le fichier [robots.txt](https://www.robotstxt.org/about.html) pour déterminer quelles pages (le cas échéant) il est autorisé à explorer.

Si vous souhaitez restreindre l'accès à une partie de votre site, ajoutez ce qui suit à votre fichier robots.txt

```
User-agent: AstianGOBot
Disallow: /private
```

Cela garantira qu'AstianGOBot n'accédera à aucune page avec le préfixe « /private ».
Vous pouvez également empêcher AstianGOBot d'accéder à n'importe quelle page de votre site.

```
User-agent: AstianGOBot
Disallow: /
```

Le fichier robots.txt est mis en cache pendant 1 heure, les modifications apportées au fichier doivent donc être respectées assez rapidement.

## Contactez-nous

Si vous avez des inquiétudes ou de mauvaises expériences avec notre robot d'exploration, n'hésitez pas à nous contacter à [contact@astian.org](mailto:contact@astian.org). Il y a de fortes chances que d'autres rencontrent les mêmes problèmes et nous serions ravis de les résoudre.