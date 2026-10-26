# Crawling: robots.txt, sitemaps and routes

[`examples/crawl.ts`](../crates/bindings/pestle-node/examples/crawl.ts) puts
both together: read a site's robots.txt, follow its sitemaps as they stream
in, and list every page it lets you fetch.

## robots.txt

RFC 9309, `parseRobots(text, token)`:

```ts
import { parseRobots, Robots } from "@pestle-rs/node";

const res = await fetch(new URL("/robots.txt", site));
const robots = res.ok
  ? parseRobots(await res.text(), "mybot")
  : res.status < 500 ? Robots.allowAll() : Robots.disallowAll(); // a 4xx: no rules; a 5xx: stay out

robots.allows("https://venue.example/agenda?page=2"); // true / false, ~200 ns a call
robots.allowsAll(urls);                               // many in one call
robots.crawlDelay;                                    // seconds, when stated
robots.sitemaps;                                      // every Sitemap: URL
```

- The groups that name your product token (by its start, so `festimap`
  names `festimapbot`), else `*`, merged; the longest matching rule wins,
  `Allow` on a tie; `*` and `$`.
- Percent-encoding normalised on both sides (`%7E` is `~`, `café` is
  `caf%C3%A9`); `Crawl-delay` (the largest); every `Sitemap:` (absolute
  http(s) URLs, as sitemaps.org requires).
- What sites get wrong is read as Google reads it: misspelled fields
  (`Dissallow`, `User agent`), a BOM, any line ending, patterns without
  their leading `/`, files past 500 KiB.
- `allows` is a linear wildcard match (no regex).

## Sitemaps

sitemaps.org, `parseSitemap(source, { url })`: an async generator over a
`fetch` response, a stream or bytes, holding only the entry in progress.

```ts
import { parseSitemap, readSitemap, NotASitemap } from "@pestle-rs/node";

for await (const entry of parseSitemap(await fetch(url))) {
  if (entry.kind === "sitemap") queue.push(entry.url); // an index names another sitemap
  else pages.push(entry);                              // { url, lastmod?, changefreq?, priority?, images?, alternates?, title? }
}
```

- It reads `<urlset>` and `<sitemapindex>` (any namespace prefix), text
  sitemaps, and the feeds sitemaps.org allows in their place (RSS 2.0,
  RSS 1.0, Atom).
- Gzip is undone by its magic bytes whatever the file is called, with a
  limit on what it may grow to (`maxBytes`, 256 MiB: a gzip bomb stops
  there), and the charset is the file's.
- URLs are absolute http(s) without `#…` (relative ones resolve against the
  sitemap's URL); invalid dates, frequencies and priorities are left out;
  `<image:loc>` and `<xhtml:link>` are never taken for the `<loc>`.
- A body that is no sitemap (an HTML page, a soft 404, an empty body)
  throws `NotASitemap`.
- `readSitemap(body)` reads one whole (`{ format, entries, gzip,
  warnings }`), `readSitemapAsync(body)` on the thread pool.

## Routes

`inferRoutes(urls)` turns a site's URLs (a sitemap's entries, the links of
its pages) into the tree of routes they follow, as Next.js writes them, with
how many URLs each holds and which ones hold one item a page: the events,
products or articles you are after.

```ts
import { inferRoutes, RouteBuilder, parseSitemap } from "@pestle-rs/node";

const b = new RouteBuilder();
for await (const e of parseSitemap(await fetch(url))) b.add(e); // or inferRoutes(urls) at once
const routes = b.finish();

routes.outline();                 // the tree as text (below)
routes.items()[0];                // { pattern: "/programma/[slug]/[id]", pages: 3086, kind: "item", topic: "event", score: 1, examples: [...] }
routes.find(link)?.kind;          // which route a URL is: "item", "listing", "pagination", …
routes.findAll(links);            // many at once: which links on a page to follow
routes.sites[0].root;             // the tree: { segment, param?, pattern, pages, total, kind, topic?, score, examples?, lastmod?, images?, children? }
```

Paradiso's sitemaps (4,894 URLs, read in 11 ms), trimmed:

```
https://www.paradiso.nl (4,894 URLs)
                                     pages   in all
/                                        1    4,894  root
├── programma                            0    3,087  section · event
│   ├── [slug]                           0    3,086  section · event
│   │   └── [id]                     3,086    3,086  item · event · 1.00  e.g. /programma/trockener-kecks/2940733
│   └── archive/2692354                  1        1  page · event
├── nieuws                               1    1,630  section · article
│   └── [slug]                           0    1,629  section · article
│       └── [id]                     1,629    1,629  item · article · 1.00  e.g. /nieuws/het-regent-prachtige-platen-tijdens-de-herft/2945686
├── info                                 0       51  section
│   ├── over-ons                         1       22  listing
│   │   └── [slug]                      21       21  item · 0.67  e.g. /info/over-ons/paradiso-r-d
…
├── contact                              1        1  page
└── vacatures                            1        1  page · job
```

`pages` counts the URLs that are a route, `in all` the URLs at it or below
it. Each URL counts once, at the route `find` gives it.

How siblings become a parameter:

- **Values with a type**: years, dates, numbers, UUIDs and hash ids, from
  two (`/news/[year]/[month]`, `/p/[id]`); a language code first in the
  path (`/[locale]/…`). Years among other numbers are numbers.
- **Slugs**: of three words or more, or with digits, from two
  (`nora-live-in-paradiso`); of two words from `minSiblings` (5); single
  words from four times that. At the top of a site, or of a language
  version, slugs need `minSiblings` and words a great many: that is where a
  site keeps its pages (`/about`, `/contact`).
- **Under a section that says what it holds** (`events`, `agenda`,
  `programma`, `producten`, `tag`, `category`, …, in English, Dutch, German,
  French and Spanish) or under another value, any two names are values.
- **Not values**: words a site keeps for its structure (`page`, `tag`,
  `archive`, `search`, `contact`, …), and under a value, the few names each
  value has (`/artists/[slug]/bio`).

Each route gets a `kind`: `item` (one item a page), `listing` (a page above
items), `taxonomy` (a page a tag, category, genre or author), `archive` (a
page a year, month or day), `pagination`, `page`, `section` or `root`; a
`topic` from the words of its path (`event`, `product`, `article`,
`person`, `venue`, `job`, `recipe`); and, for items, a `score` from 0 to 1:
how many pages it has, how many end in a value (a long slug, a number, an
id), how many are leaves, and whether its section names a topic. Given
sitemap entries, a route also has its pages' latest `lastmod` and how many
have images.

It reads one host a site (`www.` or not, each site its own tree), keeps up
to `maxUrls` (1,000,000), and takes about 1.5 µs a URL: a million URLs in
1.5 s and some 500 MB, most of it the URLs themselves.

## Parity

Both are held to festimap's `robots.ts` and `sitemap.ts` by differential
tests: the same decisions on 6,000 random robots checks, the same entries on
150 random sitemaps read in chunks of every size, plain and gzipped.

| Node | pestle | festimap (TypeScript) |
| --- | --- | --- |
| 100,000 robots.txt checks | 23 ms | 144 ms (6×) |
| a 50,000-URL sitemap (12.4 MB), streamed | 70 ms | 123 ms (1.7×) |
| the same gzipped (0.5 MB) | 78 ms, gunzipped natively | 127 ms with zlib |
