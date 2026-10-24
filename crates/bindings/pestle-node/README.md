<div align="center">

<img alt="pestle" src="https://raw.githubusercontent.com/the-haus/pestle-rs/main/docs/assets/pestle-mark.svg" width="80" height="80">

# @pestle-rs/node

**Fast streaming HTML extraction for scrapers and AI agents.**

CSS selectors, page metadata, links, embedded JSON, every kind of structured data, accessibility snapshots, robots.txt and sitemaps: read in one pass over the bytes as they arrive, in any charset. Rust under the hood, no DOM.

<p>
  <a href="https://www.npmjs.com/package/@pestle-rs/node"><img alt="npm" src="https://img.shields.io/npm/v/@pestle-rs/node?label=npm&logo=npm"></a>
  <a href="https://github.com/the-haus/pestle-rs/actions/workflows/node.yml"><img alt="node" src="https://github.com/the-haus/pestle-rs/actions/workflows/node.yml/badge.svg"></a>
  <img alt="Node.js >= 18" src="https://img.shields.io/node/v/@pestle-rs/node?logo=nodedotjs&logoColor=white">
  <img alt="types: included" src="https://img.shields.io/badge/types-included-3178c6?logo=typescript&logoColor=white">
  <a href="https://github.com/the-haus/pestle-rs#license"><img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue"></a>
</p>

</div>

```ts
import { Extractor } from "@pestle-rs/node";

const agenda = new Extractor({
  events: {
    select: "article.event",
    all: true,
    fields: {
      title: "h2",
      url: { select: "h2 a", attr: "href", url: "http" },
      when: ".date",
      price: { select: ".price", number: true },
      acts: ["ul.acts > li"],
    },
  },
}, { structured: true });

const { data, structured } = await agenda.stream(await fetch("https://venue.example/agenda"));
// data.events[0]    → { title: "Nora & Co", url: "https://venue.example/e/nora", when: "za 14 nov 2026 · 20:00", price: 25, acts: ["Nora", "Kofi", "Ada"] }
// structured.events → the page's own events (JSON-LD, microdata, RDFa, microformats), merged and flat
```

## Contents

- [Why pestle](#why-pestle)
- [Install](#install)
- [Recipes](#recipes): [a listing](#extract-a-listing-with-a-spec) · [any page](#read-any-page-without-a-spec) · [structured data](#structured-data) · [huge pages](#huge-pages-item-by-item) · [agents](#pages-for-ai-agents) · [crawling](#crawling-robotstxt-and-sitemaps) · [model prompts](#pages-for-a-models-prompt)
- [Cookbook](#cookbook): [HTTP clients](#with-any-http-client) · [many pages](#many-pages-at-once) · [pagination](#follow-pagination) · [Next.js data](#nextjs-and-other-embedded-json) · [products](#a-product-page) · [bot walls](#bot-walls) · [Playwright](#with-playwright) · [specs from a model](#specs-written-by-a-model) · [timeouts](#timeouts-and-cancellation) · [testing specs](#testing-your-specs)
- [API](#api)
- [The spec](#the-spec)
- [Errors and limits](#errors-and-limits)
- [Troubleshooting](#troubleshooting)
- [Performance](#performance)
- [Platforms](#platforms)
- [Examples](#examples)

## Why pestle

- **One pass, no DOM.** The page is tokenized and matched in Rust as it streams in. Memory is the stack of open elements plus what you asked for, so a 10 MB listing never sits in the JS heap.
- **Fast.** 6–28× faster than htmlparser2 + css-select, cheerio, linkedom or node-html-parser on the same extraction; `extractAsync` runs on libuv's thread pool and leaves the JS thread free ([numbers](#performance)).
- **Faithful.** The tree is htmlparser2's and the matches are css-select's, so selectors mean what they mean in cheerio. Differential tests check this on thousands of broken documents.
- **Everything a scraper reads.** Title, canonical, robots, links with text, lazy images, feeds, embedded `__NEXT_DATA__`, bot walls, and every JSON-LD, microdata, RDFa and microformats item as JSON-LD of one shape, conformant with the W3C suites.
- **Ready for agents.** Playwright-compatible accessibility snapshots without a browser, refs with CSS selectors, and hidden prompt-injection text left out.
- **Never throws on a page.** Malformed markup gives what can be read; every reader is bounded and says what it cut.
- **Any charset.** Shift_JIS, windows-1252, GBK and the rest, found the way browsers find them, streaming.

## Install

```sh
npm install @pestle-rs/node
# or: yarn add @pestle-rs/node · pnpm add @pestle-rs/node
```

A prebuilt native binary for your platform comes with it (an optional dependency such as `@pestle-rs/node-linux-x64-gnu`): no compiler, no postinstall script. Node.js 18 or newer; ESM and CommonJS; TypeScript types included.

```ts
import { scrape, type Page } from "@pestle-rs/node"; // ESM and TypeScript
const { scrape } = require("@pestle-rs/node");      // CommonJS
```

Check that it works:

```sh
node -e 'console.log(require("@pestle-rs/node").extractPage("<title>It works</title>").title)'
```

TypeScript needs nothing extra: the types ship in the package and resolve under `"moduleResolution": "NodeNext"`, `"Node16"` or `"Bundler"`.

## Recipes

### Extract a listing with a spec

A spec is plain JSON: fields name CSS selectors and say how each value is read. Compile it once with `new Extractor(spec)` and reuse it on any number of pages, concurrently.

```ts
import { Extractor } from "@pestle-rs/node";

const x = new Extractor({
  title: "h1",
  image: "meta[property='og:image']@content",         // "selector@attribute"
  events: {
    select: "article.event",
    all: true,
    fields: {
      title: "h2",
      url: { select: "h2 a", attr: "href", url: "http" }, // absolute, http(s) only, no #fragment
      acts: ["ul.acts > li"],                             // [selector]: every match
      headliner: "ul.acts > li:first-child",
      price: { select: ".price", number: true },          // "€ 25,-" → 25
      soldOut: { select: ".sold-out", exists: true },     // true / false
    },
  },
});

const fromString = x.extract(html, { url: "https://venue.example/agenda" });         // on this thread
const fromPool = await x.extractAsync(bytes, { url: "https://venue.example/agenda" }); // on the thread pool
const fromNet = await x.stream(await fetch("https://venue.example/agenda"));           // as it arrives; URL and charset from the response
```

Each call returns `{ data, errors, charset, stats }`, plus `page`, `structured`, `aria`, `skeleton` and `stripped` when those passes are switched on in the options. TypeScript users can type `data` with `new Extractor<MyShape>(spec)`.

### Read any page without a spec

```ts
import { scrape, extractPage } from "@pestle-rs/node";

const { page, structured } = await scrape(await fetch(url)); // the page and its structured data, one pass
if (page.blocked) retryLater(page.blocked);                  // "cloudflare", "datadome", "captcha" …

page.title; page.description; page.lang; page.canonical;
page.robots;     // { index: true, follow: true, directives?: [...] }
page.links;      // [{ href, text, rel?, nofollow? }], absolute, deduplicated
page.images;     // [{ src, alt?, width?, height? }], lazy-loaded ones by their real address
page.feeds;      // RSS, Atom, JSON feeds
page.embedded;   // [{ name: "__NEXT_DATA__", value: {...} }, { name: "window.__INITIAL_STATE__", ... }]
page.meta;       // every <meta>, by key: og:*, twitter:*, ...

const quick = extractPage(html, { url, links: false, images: false }); // sync, only what you need
```

The full field list is in [docs/page.md](https://github.com/the-haus/pestle-rs/blob/main/docs/page.md).

### Structured data

Every JSON-LD, microdata, RDFa (OpenGraph, Dublin Core) and microformats item, of every type, comes out as JSON-LD of one shape. The parser repairs what sites get wrong: broken JSON, missing `@context`, `Start_Date`-style names, relative URLs.

```ts
import { extractStructured } from "@pestle-rs/node";

const { items, events, warnings } = extractStructured(html, { url });
// items[0]  → { "@context": "https://schema.org", "@type": "MusicEvent", "@format": "json-ld", "name": "Nora & Co",
//               "startDate": "2026-11-14T20:00", "location": { "@type": "Place", "name": "Paradiso" }, ... }
// events[0] → { name, startDate, location: [{ name, address, lat, lon }], performers, offers, format, formats? }

const products = extractStructured(html, { url, types: ["Product"] }).items; // subtypes count
const strict = extractStructured(html, { url, lenient: false });           // exactly per spec, no repairs
```

`lenient: false` follows the specifications to the letter. It passes all 443 in-scope W3C JSON-LD 1.1 toRdf tests (jsonld.js passes 434), RDFa 1.1 170/170, and 139 of 140 microformats tests. Details: [docs/structured-data.md](https://github.com/the-haus/pestle-rs/blob/main/docs/structured-data.md).

### Huge pages, item by item

A top-level field with `emit: true` keeps nothing: each item is handed to you as soon as its element closes.

```ts
const x = new Extractor({
  events: { select: "article.event", all: true, emit: true, fields: { title: "h2", when: ".date" } },
});

await x.stream(await fetch(url), {
  onItem: ({ field, value }) => db.insert(value),  // called while the page is still downloading
  signal: AbortSignal.timeout(30_000),
});
```

Or feed it by hand with `session()` (`write(chunk)`, `take()`, `end()`), for example from a crawler's own HTTP client. `stream()` accepts a `fetch` `Response`, a web `ReadableStream`, a Node stream, or any (async) iterable of strings or bytes.

### Pages for AI agents

`ariaSnapshot` returns the page's accessibility tree as YAML, the same snapshot Playwright's `page.ariaSnapshot()` takes, without starting a browser. In `ai` mode every element an agent can act on gets a ref, and each ref maps to a CSS selector.

```ts
import { ariaSnapshot } from "@pestle-rs/node";

const { snapshot, refs } = ariaSnapshot(html, { url, mode: "ai", concealed: true });
// - navigation "Main" [ref=e2]:
//   - link "Agenda" [ref=e3] [cursor=pointer]:
//     - /url: /agenda
// - searchbox "Zoek een concert" [ref=e9]
// - button "Zoek" [ref=e10]
refs.e10; // "body > main:nth-child(2) > form:nth-child(2) > button:nth-child(2)": hand it to Playwright to click
```

`concealed: true` also leaves out text the page hides from sight but keeps in its markup (`font-size: 0`, off-screen, `d-none`, …), which is where prompt injections hide. The default mode is identical to Playwright's on 36 of 36 real pages, and 44× faster than Playwright on an already-loaded page. Add `aria: true` to any `Extractor` or `scrape` call to get the snapshot in the same pass. Details: [docs/aria.md](https://github.com/the-haus/pestle-rs/blob/main/docs/aria.md).

### Crawling: robots.txt and sitemaps

```ts
import { parseRobots, parseSitemap, Robots } from "@pestle-rs/node";

const res = await fetch(new URL("/robots.txt", site));
const robots = res.ok ? parseRobots(await res.text(), "mybot")
  : res.status < 500 ? Robots.allowAll() : Robots.disallowAll();

for (const sitemap of robots.sitemaps) {
  for await (const entry of parseSitemap(await fetch(sitemap))) { // gzip, indexes, RSS/Atom, text, any charset
    if (entry.kind === "sitemap") queue.push(entry.url);
    else if (robots.allows(entry.url)) pages.push(entry);        // { url, lastmod?, changefreq?, priority?, images?, alternates? }
  }
}
```

robots.txt follows RFC 9309 and reads misspellings the way Google does. Sitemaps stream with a gzip-bomb limit and throw `NotASitemap` on soft 404s. Details: [docs/crawling.md](https://github.com/the-haus/pestle-rs/blob/main/docs/crawling.md).

### Pages for a model's prompt

```ts
import { strip, skeleton, templateHash } from "@pestle-rs/node";

const { html: small, tokens, truncated } = strip(html, { maxChars: 24_000 }); // markup kept, noise out, within budget
const { hash } = skeleton(html);                                              // the page's template fingerprint
```

## Cookbook

Short answers to "how do I…". Every recipe here is checked by
[`test/cookbook.test.ts`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/test/cookbook.test.ts).

### With any HTTP client

pestle reads strings, bytes, `fetch` responses and streams. Pass bytes, not decoded text, plus the charset from the `Content-Type` header, so pages that aren't UTF-8 come out right:

```ts
import { request } from "undici";
import axios from "axios";
import { createReadStream } from "node:fs";
import { Extractor, scrape, extractPage, charsetOf } from "@pestle-rs/node";

const x = new Extractor({ events: { select: "article.event", all: true, fields: { title: "h2" } } });

// fetch: the URL and charset come from the response, and the body is read as it arrives
const { data } = await x.stream(await fetch(url));

// undici: the body is a stream
const { body, headers } = await request(url);
const { page } = await scrape(body, { url, charset: charsetOf(headers["content-type"] as string) });

// axios: ask for bytes
const res = await axios.get<ArrayBuffer>(url, { responseType: "arraybuffer" });
const info = extractPage(new Uint8Array(res.data), { url, charset: charsetOf(res.headers["content-type"] as string | undefined) });

// a saved page on disk, streamed
const saved = await x.stream(createReadStream("page.html"), { url: "https://venue.example/agenda" });
```

### Many pages at once

`extractAsync` runs on libuv's thread pool, so pages are parsed in parallel while the JS thread keeps serving requests:

```ts
const results = await Promise.all(
  urls.map(async (url) => {
    const res = await fetch(url);
    const bytes = new Uint8Array(await res.arrayBuffer());
    return x.extractAsync(bytes, { url, charset: charsetOf(res.headers.get("content-type")) });
  }),
);
```

The pool has 4 threads by default. For a parsing-heavy crawler, raise it before Node starts: `UV_THREADPOOL_SIZE=16 node crawler.js`.

### Follow pagination

The page pass reads `rel="next"` and `rel="prev"` from `<link>` and `<a>`, already absolute:

```ts
interface Listing {
  events: { title: string | null; url: string | null }[];
}
const listing = new Extractor<Listing>(
  { events: { select: "article.event", all: true, fields: { title: "h2", url: { select: "h2 a", attr: "href", url: "http" } } } },
  { page: { links: false, images: false } }, // only what we need of the page pass
);

const events: Listing["events"] = [];
for (let url: string | undefined = start, n = 0; url && n < 50; n++) {
  const { data, page } = await listing.stream(await fetch(url));
  events.push(...data.events);
  url = page?.next;
}
```

### Next.js and other embedded JSON

Many sites ship their data as JSON inside a script. `page.embedded` finds it for you: `__NEXT_DATA__`, `__NUXT_DATA__`, `<script type="application/json">`, `window.__INITIAL_STATE__ = {…}`, and `JSON.parse('…')`:

```ts
const { embedded } = extractPage(html, { url });
const next = embedded?.find((e) => e.name === "__NEXT_DATA__")?.value as { props: { pageProps: unknown } } | undefined;
console.log(next?.props.pageProps);
```

### A product page

Shops describe their products in schema.org. You get the same shape whether the page used JSON-LD, microdata or RDFa:

```ts
const [product] = extractStructured(html, { url, types: ["Product"] }).items;
product?.name;                  // "Tee"
product?.offers;                // { "@type": "Offer", price: "19.95", priceCurrency: "EUR", availability: "InStock" }
```

Values are left as the page writes them (`"19.95"`), so you decide how to parse prices and dates.

### Bot walls

```ts
const { page } = await scrape(await fetch(url));
if (page.blocked) {
  // "cloudflare", "datadome", "perimeterx", "incapsula", "akamai" or "captcha"
  await sleep(backoff);
  return retryLater(url, page.blocked);
}
```

### With Playwright

Let a browser render pages that scripts build, then read the HTML with pestle. That's far faster than querying through the browser:

```ts
import { chromium } from "playwright";
import { Extractor, ariaSnapshot } from "@pestle-rs/node";

const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto(url);
const { data } = new Extractor(spec).extract(await page.content(), { url: page.url() });
```

Or let an agent pick what to click from pestle's snapshot, and act on it in the browser through the ref's CSS selector:

```ts
const { snapshot, refs } = ariaSnapshot(await page.content(), { mode: "ai", concealed: true });
const ref = await askModel(snapshot);   // e.g. "e10", for: - button "Zoek" [ref=e10]
await page.locator(refs[ref]!).click();
```

### Specs written by a model

Specs are plain JSON, so a model can write them. Compile them with `lenient: true`: fields that don't compile are left out and listed, instead of throwing:

```ts
import { Extractor, SpecError, type Spec } from "@pestle-rs/node";

const spec = JSON.parse(modelOutput) as Spec;
const x = new Extractor(spec, { lenient: true });
if (x.warnings.length) feedBackToModel(x.warnings);
// ["bad: selector: \"article:has(.x) h2\": :has() can only be tested on the selected element itself",
//  "worse: selector: expected a name at 3 in \"h1[\""]

// or strictly, with every problem at once:
try {
  new Extractor(spec);
} catch (e) {
  if (e instanceof SpecError) console.error(e.errors);
}
```

`strip(html)` gives the model a compact version of the page to write the spec from (about 12k tokens by default). After extraction, `errors` and `stats.matches` show whether the spec still fits the page.

### Timeouts and cancellation

```ts
const signal = AbortSignal.timeout(10_000);
const { data } = await x.stream(await fetch(url, { signal }), { signal }); // rejects with a TimeoutError
```

### Testing your specs

Save a page once, and test the spec against it as you'd test any function:

```ts
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { agenda } from "../src/specs.js"; // your `new Extractor(...)`

it("reads the agenda", () => {
  const { data, errors } = agenda.extract(readFileSync("fixtures/agenda.html"), { url: "https://venue.example/agenda" });
  expect(errors).toEqual([]);
  expect(data.events[0]).toMatchObject({ title: "Nora & Co", price: 25 });
});
```

## API

| | |
| --- | --- |
| **`new Extractor(spec, options?)`** | Compile a [spec](#the-spec). Options: `structured`, `page`, `aria`, `skeleton`, `strip` (each `true` or options), `lenient`, `maxDepth`, `maxCapture`, `grammars`. |
| `extractor.extract(html, { url?, charset? })` | A whole document (string or bytes), on this thread. |
| `extractor.extractAsync(html, { url?, charset? })` | The same on libuv's thread pool. |
| `extractor.stream(source, { url?, charset?, onItem?, signal? })` | A document as it arrives. A `fetch` response gives its own URL and charset. |
| `extractor.session({ url?, charset? })` | Feed it yourself: `write(chunk)`, `take()`, `end()`. |
| `extractor.warnings` | Fields a `lenient` spec left out, with the reason. |
| **`scrape(source, { url?, charset?, page?, structured?, aria?, signal? })`** | `{ page, structured, aria?, charset }` in one pass. |
| `extractPage(html, options?)` / `extractPageAsync` | The [page pass](https://github.com/the-haus/pestle-rs/blob/main/docs/page.md) alone. |
| `extractStructured(html, options?)` / `extractStructuredAsync` | `{ items, events, raw?, warnings }`. |
| `ariaSnapshot(html, options?)` / `ariaSnapshotAsync` | `{ snapshot, refs }`. Options: `mode`, `depth`, `concealed`, `styleBlocks`. |
| **`parseRobots(text, token)`** | A `Robots`: `allows(url)`, `allowsAll(urls)`, `rules`, `crawlDelay`, `sitemaps`. `Robots.allowAll()`, `Robots.disallowAll()`. |
| `parseSitemap(source, options?)` | An async generator of `SitemapEntry`. Options: `url`, `maxBytes`, `maxEntries`, `maxEntryBytes`. |
| `readSitemap(body, options?)` / `readSitemapAsync` | A whole sitemap: `{ format, entries, gzip, warnings? }`. |
| **`strip(html, options?)`** | `{ html, tokens, truncated }`: festimap's `cleanForLearn`. |
| `skeleton(html, options?)`, `templateHash(pathsPerPage)` | The template fingerprint. |
| `charsetOf(contentType)` | `"text/html; charset=ISO-8859-1"` → `"ISO-8859-1"`. |
| `hasGrammars()` | Whether this build includes pest grammars (`Grammar`, `parse` fields). The published build does not; see [grammars](https://github.com/the-haus/pestle-rs/blob/main/docs/grammars.md). |

Errors: `SpecError` (a spec that does not compile, with `errors[]`), `NotASitemap`, and with grammars `GrammarError` and `ParseError`. Every option and result is typed: `Spec`, `FieldSpec`, `ExtractResult<T>`, `Page`, `Structured`, `StructuredItem`, `StructuredEvent`, `AriaSnapshot`, `SitemapEntry`, and the rest.

## The spec

A field is a selector string, `[selector]` for every match, `"selector@attr"` for an attribute, or an object:

| key | what |
| --- | --- |
| `select` | CSS selector, relative to the field's scope (absent: the scope element itself) |
| `all`, `limit` | every match as an array, at most `limit` |
| `attr` | read this attribute instead of the text |
| `text` | `"visible"` (default: as a reader sees it), `"raw"` (as is, scripts too), `"own"` (only the element's own text) |
| `html` | the element's outer HTML |
| `exists` | `true` when something matches |
| `url` | resolve against the page (`true`), or as an http(s) page without `#…` (`"http"`) |
| `pattern` | a regex: its first group, else its match |
| `number` | read as a number (`€ 25,-` → 25, `1.234,50` → 1234.5) |
| `json` | parse as JSON |
| `max`, `collapse` | cut to this many characters; collapse whitespace |
| `fields` | an object per match, its fields relative to the match |
| `default` | the value when nothing matched (else `null`, or `[]` with `all`) |
| `emit` | top-level with `all`: hand items over as they complete |

Selectors: type, `*`, `#id`, `.class`, every attribute operator (with `i`/`s`), all combinators, `:not()`, `:is()`, `:where()`, `:nth-child()`, `:nth-of-type()`, `:first-*`, `:root`, `:scope`; and on the selected element itself `:has()`, `:contains()`, `:empty`, `:last-child`, `:only-child`, `:nth-last-*`. Full reference: [docs/spec.md](https://github.com/the-haus/pestle-rs/blob/main/docs/spec.md).

## Errors and limits

- A page never makes pestle throw. A value that can't be read (bad JSON, say) is `null`, and `errors` says why, with the field's path.
- A spec that doesn't compile throws `SpecError` listing every problem. `lenient: true` leaves those fields out instead (`extractor.warnings`), which helps when specs come from a model.
- Bounds: `maxDepth` (1,024 levels; deeper tags are ignored) and `maxCapture` (4 MiB per value). The page, structured-data and snapshot passes have caps of their own and list what they cut in `warnings`.
- `stream()` and `scrape()` take an `AbortSignal`.

## Troubleshooting

**A field is `null`.** Check `errors` first: a value that couldn't be read (bad JSON, say) is listed there with its field. If `errors` is empty, the selector matched nothing. `stats.matches` counts the elements any field matched, and `extractPage(html).headings` or `ariaSnapshot(html).snapshot` show what's actually in the HTML. Pages that scripts build have little in their HTML: render them first ([with Playwright](#with-playwright)), or read their [embedded JSON](#nextjs-and-other-embedded-json).

**`SpecError: … :has() can only be tested on the selected element itself`.** Conditions that depend on what comes later (`:has()`, `:last-child`, `:contains()` …) work on the selected element, not on an ancestor. Instead of `"article:has(.sold-out) h2"`, write `{ select: "article:has(.sold-out)", fields: { title: "h2" } }`.

**Garbled characters (`CafÃ©`).** You passed text that was decoded with the wrong charset. Pass the bytes (a `Uint8Array`, a `Buffer`, a stream or the `fetch` response itself) and let pestle find the encoding, or give it the server's charset with `charsetOf(contentType)`.

**`no native binary found for <platform>`.** The optional dependency for your platform wasn't installed. Usually the lockfile was made on another OS, or `--no-optional` / `--omit=optional` was used. Reinstall on the target machine (`rm -rf node_modules && npm install`). In Docker, run `npm ci` inside the image, not on the host.

**Bundlers and frameworks.** A native addon can't be bundled; mark the package as external:

| | |
| --- | --- |
| Next.js | `serverExternalPackages: ["@pestle-rs/node"]` in `next.config.js` |
| esbuild | `--external:@pestle-rs/node` |
| Vite (SSR) | `ssr: { external: ["@pestle-rs/node"] }` |
| webpack | `externals: { "@pestle-rs/node": "commonjs @pestle-rs/node" }` |
| Electron | unpack `**/*.node` from the asar archive (`asarUnpack` in electron-builder) |

To ship the binary yourself, point `PESTLE_RS_NODE_BINARY` at the `.node` file.

**The event loop stalls on big pages.** `extract`, `extractPage`, `extractStructured` and `ariaSnapshot` run on the calling thread. In a server, use `extractAsync`, `extractPageAsync`, `extractStructuredAsync`, `ariaSnapshotAsync` or `stream`, which run on the thread pool.

**Memory grows on huge listings.** Without `emit`, an `all` field keeps every item until the end. Add `emit: true` to the top-level field and handle items in `onItem` ([huge pages](#huge-pages-item-by-item)).

## Performance

M-series Mac, Node 24, 9 real venue and festival pages (4.2 MB), the same extraction (links with text, headings, images, title):

| | MB/s | |
| --- | --- | --- |
| **@pestle-rs/node** | **703** | 1,056 MB/s with `extractAsync` on the thread pool |
| htmlparser2 SAX, hand-written | 142 | 5.0× slower |
| node-html-parser | 110 | 6.4× slower |
| htmlparser2 + css-select | 107 | 6.6× slower |
| linkedom | 44 | 16× slower |
| cheerio | 25 | 28× slower |

More: [docs/performance.md](https://github.com/the-haus/pestle-rs/blob/main/docs/performance.md).

## Platforms

Prebuilt for Linux x64 and arm64 (glibc 2.31+ and musl/Alpine), macOS x64 and arm64, and Windows x64 and arm64. Each binary is tested on its own platform, and the packed tarball is installed and used on each one before release.

- **Bundlers and Electron.** Keep `@pestle-rs/node` external, since it loads a `.node` file. To ship the binary somewhere else, set `PESTLE_RS_NODE_BINARY` to its path.
- **Other native addons.** pestle loads beside other addons in the same process. On Linux and Windows it uses mimalloc. On macOS it uses the system allocator, because two addons that each bundle mimalloc crash each other there.
- **No binary for your platform?** Build from source: clone [the repository](https://github.com/the-haus/pestle-rs), then `corepack yarn install && corepack yarn workspace @pestle-rs/node build` (needs Rust).

## Examples

Runnable TypeScript in [`examples/`](https://github.com/the-haus/pestle-rs/tree/main/crates/bindings/pestle-node/examples), also shipped in the package (`node_modules/@pestle-rs/node/examples`). Each runs with `npx tsx`, or with plain `node` on Node 22.18+, and works offline on the bundled [`agenda.html`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/agenda.html):

| | |
| --- | --- |
| [`scrape.ts`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/scrape.ts) | everything a page says about itself, plus its structured data |
| [`extract.ts`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/extract.ts) | a spec from a JSON file ([`agenda.spec.json`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/agenda.spec.json)) on a URL or a saved page |
| [`typed-spec.ts`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/typed-spec.ts) | a spec with a typed result, used like any typed data |
| [`stream-items.ts`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/stream-items.ts) | 100,000 events streamed with `emit`, in about 12 MB of heap |
| [`agent-input.ts`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/agent-input.ts) | an `ai`-mode snapshot with hidden instructions left out |
| [`crawl.ts`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/crawl.ts) | robots.txt plus sitemap indexes, streamed |
| [`commonjs.cjs`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/commonjs.cjs) | the same API from `require` |

```sh
cd node_modules/@pestle-rs/node/examples
npx tsx scrape.ts https://example.com
npx tsx extract.ts agenda.spec.json agenda.html https://venue.example/agenda
npx tsx typed-spec.ts
```

## License

MIT OR Apache-2.0. Source, issues and changelog: [github.com/the-haus/pestle-rs](https://github.com/the-haus/pestle-rs).
