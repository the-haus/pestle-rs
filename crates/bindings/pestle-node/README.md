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
- [API](#api)
- [The spec](#the-spec)
- [Errors and limits](#errors-and-limits)
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

```js
import { scrape } from "@pestle-rs/node";      // ESM
const { scrape } = require("@pestle-rs/node"); // CommonJS
```

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

Runnable scripts in [`examples/`](https://github.com/the-haus/pestle-rs/tree/main/crates/bindings/pestle-node/examples), also shipped in the package (`node_modules/@pestle-rs/node/examples`):

| | |
| --- | --- |
| [`scrape.mjs`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/scrape.mjs) | everything a page says about itself, plus its structured data |
| [`extract.mjs`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/extract.mjs) | a spec from a JSON file ([`agenda.spec.json`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/agenda.spec.json)) on a URL or a saved page |
| [`stream-items.mjs`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/stream-items.mjs) | 100,000 events streamed with `emit`, in about 12 MB of heap |
| [`agent-input.mjs`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/agent-input.mjs) | an `ai`-mode snapshot with hidden instructions left out |
| [`crawl.mjs`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/crawl.mjs) | robots.txt plus sitemap indexes, streamed |
| [`commonjs.cjs`](https://github.com/the-haus/pestle-rs/blob/main/crates/bindings/pestle-node/examples/commonjs.cjs) | the same API from `require` |

```sh
node node_modules/@pestle-rs/node/examples/scrape.mjs https://example.com
```

## License

MIT OR Apache-2.0. Source, issues and changelog: [github.com/the-haus/pestle-rs](https://github.com/the-haus/pestle-rs).
