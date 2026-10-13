# AstianGO 爬虫

AstianGO 是一个[开源](https://github.com/goastian/astiango/) 网络搜索引擎。 AstianGOBot 是我们的爬虫程序的名称，它从网络上收集页面以构建索引。
它是用 Rust 编写的，源代码可以在[此处](https://github.com/goastian/astiango/tree/main/crates/core/src/crawler) 找到。
爬虫使用用户代理`Mozilla/5.0 (compatible; AstianGOBot/0.2; open source search engine; +https://astiango.com/webmasters)`。

## 礼貌

AstianGOBot 是一个有礼貌的爬虫。它尊重正在抓取的网站的 [robots.txt](https://en.wikipedia.org/wiki/Robots.txt) 文件，并尝试不使服务器过载。

### 等待时间计算

爬网程序在对同一域的请求之间等待一定的时间。等待时间按下式计算：
_min((2^礼貌) \* max(获取时间, 5 秒), 180 秒)_

在哪里：

- _politeness_ 是礼貌因素（从 2 开始）
- _fetchtime_ 是获取上一页所花费的时间

爬网程序将在请求之间等待至少 5 秒，最多 180 秒。

### 429 响应处理

如果爬虫收到服务器的 429（请求过多）响应：

1. 礼貌因素增加，每次请求之间的时间加倍。
2. 这种增加的礼貌因素在该特定域的爬网期间持续存在。
3. 爬虫绝不会降低返回 429 响应的域的礼貌系数。

如果爬虫没有收到来自服务器的任何 429 响应，它可能会随着时间的推移逐渐降低礼貌系数，但永远不会低于 0。

### 机器人.txt

爬网程序会在 [robots.txt](https://www.robotstxt.org/about.html) 文件中查找令牌 AstianGOBot，以确定允许其爬网的页面（如果有）。

如果您想限制对网站部分内容的访问，请将以下内容添加到您的 robots.txt 文件中

```
User-agent: AstianGOBot
Disallow: /private
```

这将确保 AstianGOBot 不会访问任何带有“/private”前缀的页面。
您还可以限制 AstianGOBot 访问您网站上的任何页面

```
User-agent: AstianGOBot
Disallow: /
```

robots.txt 文件缓存 1 小时，因此文件的更改应该很快得到遵守。

## 联系我们

如果您对我们的抓取工具有任何疑问或不良体验，请随时通过 [contact@astian.org](mailto:contact@astian.org) 与我们联系。其他人很可能会遇到同样的问题，我们很乐意解决这些问题。