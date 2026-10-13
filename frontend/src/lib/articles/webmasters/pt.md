# Rastreador AstianGO

AstianGO é um mecanismo de pesquisa na web [código aberto] (https://github.com/goastian/astiango/). AstianGOBot é o nome do nosso rastreador que coleta páginas da web para construir o índice.
Está escrito em Rust e o código fonte pode ser encontrado [aqui](https://github.com/goastian/astiango/tree/main/crates/core/src/crawler).
O rastreador usa o agente do usuário `Mozilla/5.0 (compatible; AstianGOBot/0.2; open source search engine; +https://astiango.com/webmasters)`.

## Polidez

AstianGOBot é um rastreador educado. Ele respeita o arquivo [robots.txt](https://en.wikipedia.org/wiki/Robots.txt) do site que está rastreando e tenta não sobrecarregar o servidor.

### Cálculo do tempo de espera

O rastreador espera um determinado período de tempo entre solicitações para o mesmo domínio. O tempo de espera é calculado por:
_min((2^educação) \* max(fetchtime, 5 seg), 180 seg)_

Onde:

- _polidez_ é o fator de educação (começando em 2)
- _fetchtime_ é o tempo que levou para buscar a página anterior

O rastreador aguardará pelo menos 5 segundos entre as solicitações e no máximo 180 segundos.

### 429 Tratamento de respostas

Se o rastreador receber uma resposta 429 (muitas solicitações) do servidor:

1. O fator polidez é aumentado, duplicando o tempo entre cada solicitação.
2. Esse fator de polidez aumentado persiste durante o rastreamento desse domínio específico.
3. O rastreador nunca diminuirá o fator de polidez de um domínio que retornou uma resposta 429.

Se o rastreador não tiver recebido nenhuma resposta 429 de um servidor, ele poderá diminuir gradualmente o fator de cortesia ao longo do tempo, mas nunca abaixo de 0.

### Robôs.txt

O rastreador procura o token AstianGOBot no arquivo [robots.txt](https://www.robotstxt.org/about.html) para determinar quais páginas (se houver) ele pode rastrear.

Se você deseja restringir o acesso a parte do seu site, adicione o seguinte ao seu arquivo robots.txt

```
User-agent: AstianGOBot
Disallow: /private
```

Isso garantirá que o AstianGOBot não acesse nenhuma página com o prefixo '/private'.
Você também pode restringir o acesso do AstianGOBot a qualquer página do seu site

```
User-agent: AstianGOBot
Disallow: /
```

O arquivo robots.txt é armazenado em cache por 1 hora, portanto, as alterações no arquivo devem ser respeitadas rapidamente.

## Contate-nos

Se você tiver alguma dúvida ou experiência ruim com nosso rastreador, não hesite em entrar em contato conosco pelo e-mail [contact@astian.org](mailto:contact@astian.org). É provável que outras pessoas tenham os mesmos problemas e adoraríamos resolvê-los.