# The page pass and charsets

## Page

With `page: true` (or `extractPage`, or `scrape`), the same pass reads what
a scraper wants of any page, with no spec at all:

```ts
import { extractPage, scrape } from "@pestle-rs/node";

const page = extractPage(html, { url });
const { page, structured } = await scrape(await fetch(url)); // the page and its structured data, streamed
```

| field | from |
| --- | --- |
| `title`, `description`, `lang` | `<title>` (not an SVG's), `<meta name="description">`, `<html lang>`; else OpenGraph's, Twitter's, `content-language` |
| `canonical`, `amp`, `manifest`, `next`, `prev` | `<link rel>`; pagination from `<a rel>` too; `og:url` when there is no canonical |
| `robots` | `<meta name="robots">` and `googlebot`: `{ index, follow, directives }` |
| `refresh` | `<meta http-equiv="refresh" content="5; url=…">` |
| `meta` | every `<meta>` by its `name`, `property`, `itemprop` or `http-equiv`, lowercase: OpenGraph (also when written with `name=`), Twitter cards, everything |
| `alternates`, `feeds`, `calendars`, `icons` | `hreflang` versions; RSS/Atom/JSON feeds; `text/calendar`, `webcal:` and `.ics` links; favicons |
| `links` | every `<a href>` once, http(s), absolute, without `#…`, with its text (else its `title`, `aria-label` or an image's `alt`), `rel` and `nofollow` |
| `images` | every `<img>` once, lazy-loaded ones by their real address (`data-src`, `data-lazy-src`, `srcset`'s largest), with `alt`, `width`, `height` |
| `headings`, `frames`, `emails`, `phones` | `h1`–`h6`; `<iframe>` (maps, videos, ticket shops); `mailto:` and `tel:` links |
| `embedded` | the JSON pages keep for their scripts: `__NEXT_DATA__`, `__NUXT_DATA__`, `<script type="application/json">`, `window.__INITIAL_STATE__ = {…}`, `… = JSON.parse('…')` |
| `blocked` | a bot wall instead of the page: Cloudflare, DataDome, PerimeterX, Imperva/Incapsula, Akamai, captchas |
| `charset` | the encoding it was read in |

Everything is absolute (against `<base href>`), deduplicated, capped
(`warnings` says what was cut) and cleaned (whitespace collapsed, soft
hyphens and zero-width characters out). Template contents are skipped.

`PageOptions` turns parts off (`links`, `images`, `headings`, `embedded`)
and sets the caps: `maxLinks` (5,000), `maxImages` (2,000), `maxHeadings`
(500), `maxEmbedded` (32), `maxText` (240 characters of a link's or
heading's text).

`text: true` also gives the page's visible text as `text`, a line per
block: a newline where a heading, a paragraph, a list item, a table row
or a `<br>` breaks it, spaces collapsed within a line, empty lines left
out, nothing of the `<head>`, scripts, styles, `<template>` and
`<noscript>`, entities decoded. It is what a field reads with
`text: "lines"` ([spec](spec.md#how-values-are-read)), here for the whole
page in the pass a crawl makes anyway; `maxTextBytes` (4 MiB) bounds it.

## Charsets

Bytes are read in the encoding the page is in, found as browsers find it:

1. a byte order mark;
2. the charset the server stated (the `charset` option; `stream` and
   `scrape` take it from a `fetch` response's `Content-Type`);
3. a `<meta charset>` or `http-equiv` in the first kilobyte;
4. an XML declaration;
5. else UTF-8, or windows-1252 when the bytes plainly are not UTF-8.

Every WHATWG encoding is read (`encoding_rs`, Firefox's): Latin-1,
windows-125x, ISO-8859-x, Shift_JIS, EUC-JP, EUC-KR, GBK, Big5, KOI8,
UTF-16 … A page in UTF-8 is still read in place, without a copy; a stream
starts as soon as its encoding is known (a stated charset, a BOM or a meta,
else after its first kilobyte). JavaScript strings are text already decoded
and are read as they are.

```ts
import { charsetOf } from "@pestle-rs/node";

const res = await fetch(url);
const bytes = new Uint8Array(await res.arrayBuffer());
x.extract(bytes, { url, charset: charsetOf(res.headers.get("content-type")) }); // what stream() does for you
```

## Strip and skeleton

Two outputs festimap's pipeline uses, byte for byte:

- `strip(html, options)` is `cleanForLearn`: the page cut down to a
  budget for a model's prompt (36,000 by default, counted in UTF-16 units\n  as `string.length` counts them, festimap's measure): scripts,
  styles and boilerplate out, long text and attributes cut, alike siblings
  past four collapsed to a comment, the title, meta and JSON-LD up front.
  `{ html, tokens, truncated }`.
- `skeleton(html)` is the template fingerprint: a sha256 of the page's
  element paths (`{ hash, paths? }`), and `templateHash(pathsOfManyPages)`
  the fingerprint of the template several pages share (the paths at least
  half of them have).

Both can also run in the same pass as an extraction (`{ strip: true,
skeleton: true }` in `ExtractorOptions`).
