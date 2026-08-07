# Offline fixture site

A deliberately hostile static site for testing the crawler without touching the live web.

Every page here exists because something in [[Web Fetcher]], [[Politeness and Robots]] or
[[Content Parser]] can fail on it. Real sites do all of these things; discovering that during a
live crawl means discovering it slowly, unreproducibly, and while being rate-limited by someone
else's server.

Served by `make fixture-site` on port 8099.

| Path | What it exercises |
|:---|:---|
| `/robots.txt` | `Crawl-delay`, `Disallow`, and a sitemap reference |
| `/sitemap.xml` | Sitemap parsing and article-shape filtering |
| `/feed.xml` | RSS discovery as a fallback when the sitemap is navigational |
| `/articles/normal.html` | The happy path: clean article markup with metadata |
| `/articles/malformed.html` | Unclosed tags, stray `</div>`, nested forms |
| `/articles/windows-1256.html` | Arabic in a legacy encoding, declared only in a meta tag |
| `/articles/spa.html` | Content only present after JavaScript runs — must yield *nothing* |
| `/articles/dates.html` | Maghrebi month names (أوت, جويلية) and day-first numerics |
| `/articles/injection.html` | A passage carrying prompt-injection text, for [[Summarizer]] |
| `/redirect/1` … `/redirect/4` | A four-hop redirect chain ending at a real article |
| `/redirect/loop` | A redirect cycle |
| `/slow` | Responds after 5 seconds, for timeout handling |
| `/429` | Always `429` with `Retry-After: 2` |
| `/500` | Always `500` |
| `/trap/` | Infinite self-referential link depth, for the crawler trap guard |
| `/private/secret.html` | Disallowed in `robots.txt`; fetching it is a bug |

The `disallowed` and `trap` cases are the important ones: they are the two where a crawler that
"works" and a crawler that is a menace look identical from the outside.
