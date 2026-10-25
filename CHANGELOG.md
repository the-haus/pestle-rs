# Changelog

## 0.2.1 — 2026-10-25

Broken JSON-LD and embedded JSON read as meant in more of the ways sites break it, and never at a cost out of
proportion to the block.

- Fixed: an unquoted key with no space after its colon lost its value when the value was not quoted: `{a:1}` read as
  `{"a:1": null}`. A colon joins an unquoted key only when another colon follows (`schema:name: …`), and not a URL's.
- Fixed: a key with no value (`{"a": }`) took its object's closing brace with it, and with that everything after it.
- Fixed: a block whose quotes are entities (`{&quot;@type&quot;: …}`) is decoded once, and only when its first quote is
  one: a long chain of `&amp;amp;…` could overflow the stack, and `&quot;` in the text of a block with real quotes
  was decoded into it.
- Fixed: `window["x"] = {…}`, `"use strict";` or `x = 5;` around a block no longer give a stray first value, which
  embedded JSON took in place of the block's data.
- Fixed: a hex number past 64 bits read as `0` (now the nearest float); `\u` and `\x` escapes took a `+` as a digit; a
  lone `-` or `.` read as a string.
- Added: unquoted values are read as strings, whole: `url: https://…`, `width: 3px`, `date: 2024-10-25`, `10:30`; so
  are “smart quotes” from word processors and CMSs, and dotted unquoted keys (`og.title`).
- Tested by reading back random values written the ways sites write them (either quotes or none, comments, trailing
  and missing commas, Python literals), every prefix of such text, random token soup, and hostile blocks.

## 0.2.0 — 2026-10-24

festimap parity where `strip` cuts, block indices in `raw.jsonLd`, and size limits that hold however a page streams.

- Changed: `strip`'s budgets (`maxChars`, `maxText`, `maxAttr`, `maxJsonLd`, the `data-*` limit) and `tokens` count
  UTF-16 units, as JavaScript's `length` and festimap's `cleanForLearn` count them, not bytes or code points. Its cut
  is now festimap's wherever it falls: a `<title>`, `<meta>` or JSON-LD block late in the page takes its room from the
  start of the body, as festimap writes them first, and an empty wrapper dropped at the end still counts. Where
  festimap cuts a character past U+FFFF in half it keeps a lone surrogate; pestle writes U+FFFD, the same UTF-8.
  Held to `cleanForLearn` at budgets from 120 to 24,000 on every fixture page, with and without astral text.
- Changed: `structured.raw.jsonLd` has one entry per JSON-LD `<script>`, in page order, so a block's index is its place
  on the page: `null` for a block that cannot be read (it was left out, moving the ones after it), an array for
  several values glued together in one (they were spread out).
- Fixed: a value past a size limit could get text from after the cut spliced onto it when the page arrived in pieces:
  an `html` field past `maxCapture` (`<div><p>…` then a later `</p>`), a JSON-LD block past `max_block_bytes`, and an
  embedded JSON script past `max_script_bytes` (8 MiB each, Rust options), which could then read as JSON it never
  was. Each now stops at its limit: an `html` field holds its first `maxCapture` bytes however the page streams (and no
  longer stops the field's text), a JSON-LD block its first `max_block_bytes` (read as the truncated block it is), and
  such a script is left out.
- Fixed: `robots.txt` was read to 512 Ki *characters*, up to 2 MiB; it is now 512 KiB, as documented. `maxCapture`
  cut a value's last piece by characters too, so a value could pass it.

## 0.1.0 — 2026-10-24

The first release of `@pestle-rs/node` on npm: prebuilt for Linux (glibc and musl), macOS and Windows, x64 and
arm64, Node.js 18+, ESM and CommonJS, types included.

- Packaging: runnable TypeScript examples shipped in the package (`examples/`: scrape, extract with a spec, a typed
  spec, streamed items, agent input, crawling; and CommonJS), each run by the tests against a local site; a Node.js guide as the package README;
  a cookbook of recipes checked by `test/cookbook.test.ts`; docs in `docs/`; CI on three OSes and Node 20/22/24, the W3C and microformats conformance suites at pinned commits,
  and a release pipeline that tests every platform binary where it was built and installs the packed tarballs on
  every platform before publishing (with npm provenance).
- Fixed: `parseRobots`, `parseSitemap`, `readSitemap` and `readSitemapAsync` threw when the package was loaded with
  `import`: the ESM loader left the crawler's native exports out. `test/package.test.ts` now runs the built ESM and
  CommonJS entry points.
- Fixed: the error for a grammar API in a build without grammars pointed at a field option (`as`) that does not
  exist; `yarn shootout` stopped at its grammar part in the default build.

- Fixed: on macOS, `@pestle-rs/node` loaded in one process with `@wikidata-rs/node` (in either order) crashed with a
  segfault (exit 139). Each bundled its own mimalloc, which on macOS keeps every thread's heap in the same fixed TLS
  slots, so each took the other's heaps. The addon now uses the system allocator on macOS; mimalloc stays on Linux and
  Windows, where each copy has its own thread-local storage (`test/coexist.test.ts`).

- Accessibility snapshot (`ariaSnapshot`, `aria: true` on any pass, `pestle_html::aria`): the page's accessibility
  tree as YAML, as Playwright's `page.ariaSnapshot()` gives it, in its default and `ai` modes (refs, each with a CSS
  selector). It ports Playwright's roles, accessible names, states and distilling passes. Hidden content is read from
  the markup, inline styles and the page's own `<style>` blocks (specificity, `!important`, media queries).
  `concealed` also leaves out text hidden from sight by inline styles or framework classes. Held to Playwright 1.64.1
  on 36 pages (`fixtures/aria`): identical in default mode. 44× faster than Playwright's snapshot of an
  already-loaded page from one thread, 145× on the thread pool.

- robots.txt (`parseRobots`, RFC 9309) and sitemaps (`parseSitemap`, `readSitemap`, `readSitemapAsync`): festimap's
  semantics (held to its `robots.ts` and `sitemap.ts` by differential tests), plus Google's leniency for misspelled
  fields, streaming gzip with a size limit, any charset, RSS/Atom feeds as sitemaps, images and `hreflang`
  alternates. 7× and 1.8× faster than festimap's from Node.

- Page pass (`page: true`, `extractPage`, `scrape`): title, description, language, canonical, robots, meta refresh,
  every meta tag, alternates, feeds, calendars, icons, pagination, links with text, images (lazy-loaded too),
  headings, frames, e-mail and phone, embedded script JSON (`__NEXT_DATA__`, `window.__INITIAL_STATE__ = …`), and
  bot-wall detection (`blocked`).
- Charsets: pages are read in their encoding (BOM, the server's `charset`, `<meta charset>`, XML declaration, else
  UTF-8 or windows-1252), streaming; `stream`/`scrape` take a `fetch` response's charset and URL.

- pest is optional: `pestle-html` and `@pestle-rs/node` build without it by default (`grammar` feature,
  `yarn build:grammar`); `hasGrammars()` says which build is loaded, and grammar APIs say so when absent.
- Spec compliance, measured on the official suites: JSON-LD 1.1 (a processor of the W3C algorithms, used when
  `lenient: false`: 443/443 toRdf tests in scope, jsonld.js passes 434), RDFa 1.1 + HTML+RDFa (170/170, now with
  property copying, lists across siblings, `about="[]"`), microformats (139/140, the reference parser 125: the include pattern, value-class timezones as `-0800`, nested items' values from their `name`/`url`, an `<a>` in an `<a>` as HTML's tree has it, `rels` sorted), WHATWG
  microdata (an oracle of the algorithm on 2,000 documents: `itemid` only on typed items, the first element of an
  id for `itemref`, absent vs empty URLs). IRIs resolve per RFC 3986 as written (no normalization).
- Structured data: JSON-LD (lenient JSON, contexts, `@graph`, merge by `@id`), microdata (WHATWG + `itemref`),
  RDFa 1.1 (OpenGraph, Dublin Core), microformats2 and the classic formats, as one list of items and a flat
  events view with values as stated; the same event stated twice is merged (`formats`).
- Every item is JSON-LD of one shape whatever the format (`@context` with the prefixes it uses, schema.org short
  names, embedded once per item); failsafe for what sites get wrong (broken JSON, misspelled script types, contexts,
  types and names in any case or style, scheme-less `itemtype`, RDFa without `vocab`, HTML in JSON-LD text, empty
  values, relative URLs in `content`), with a corpus of faulty pages in `tests/edge_cases.rs`.
- Fixed: a microformat holding thousands of nested elements overflowed a 2 MB thread stack (kept trees are now at most
  256 deep).
- Fixed: RDFa merged every top-level `typeof` of a page without `<html>` into the page; microdata `url`/`image`
  without `href`/`src` took the page's address; quadratic time on pages of many items (20,000 items: 58 s → 0.2 s);
  references that multiplied when written out (each node is now embedded once per item).
- Faster structured data on real pages (about 2.3×): elements without RDFa share their parent's context, an
  `href` is resolved only when a descendant uses it, one attribute scan per element.

- `pestle-grammar`: pest grammars compiled at run time to pestle's own engine with pest's semantics (the same pairs,
  tags and errors as `pest_vm`, checked over thousands of mutated inputs), 3–5× faster than `pest_derive`; trees,
  values shaped by node tags (also straight to JSON text), search, panic-mode error recovery.
- `pestle-html`: one streaming pass with htmlparser2's tree rules and css-select's matching; fields read as text,
  attributes or markup, then URLs, regexes, numbers, JSON or grammars; nested scopes; conditions settled later
  (`:has()`, `:last-child`, …); emitted items; festimap's skeleton and `cleanForLearn`, byte for byte.
- `@pestle-rs/node`: `Grammar` (`parse` with `recover`, `shape`, `shapeAll`), `Extractor` (sync, thread pool,
  streams of any kind, sessions), `strip`, `skeleton`, `templateHash`.
