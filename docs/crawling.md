# Crawling: robots.txt and sitemaps

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

## Parity

Both are held to festimap's `robots.ts` and `sitemap.ts` by differential
tests: the same decisions on 6,000 random robots checks, the same entries on
150 random sitemaps read in chunks of every size, plain and gzipped.

| Node | pestle | festimap (TypeScript) |
| --- | --- | --- |
| 100,000 robots.txt checks | 23 ms | 144 ms (6×) |
| a 50,000-URL sitemap (12.4 MB), streamed | 70 ms | 123 ms (1.7×) |
| the same gzipped (0.5 MB) | 78 ms, gunzipped natively | 127 ms with zlib |
