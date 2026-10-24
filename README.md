<div align="center">

<a href="https://github.com/the-haus/pestle-rs">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/pestle-mark-dark.svg">
    <img alt="pestle" src="docs/assets/pestle-mark.svg" width="96" height="96">
  </picture>
</a>

# pestle

**HTML read as a stream, for scrapers and AI agents.**

Everything a crawler reads of a page (CSS selectors, metadata, links, embedded JSON, every kind of structured data, its accessibility snapshot) in one pass over the bytes as they arrive, in any charset. No DOM, never a crash.

<p>
  <a href="https://github.com/the-haus/pestle-rs/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/the-haus/pestle-rs/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/the-haus/pestle-rs/actions/workflows/node.yml"><img alt="Node" src="https://github.com/the-haus/pestle-rs/actions/workflows/node.yml/badge.svg"></a>
  <a href="https://github.com/the-haus/pestle-rs/actions/workflows/suites.yml"><img alt="Conformance suites" src="https://github.com/the-haus/pestle-rs/actions/workflows/suites.yml/badge.svg"></a>
  <a href="https://github.com/the-haus/pestle-rs/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/the-haus/pestle-rs?label=release&color=1f6f5c"></a>
  <a href="https://www.npmjs.com/package/@pestle-rs/node"><img alt="npm" src="https://img.shields.io/npm/v/@pestle-rs/node?label=npm&logo=npm"></a>
  <a href="#license"><img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue"></a>
</p>

<sub>Streaming · no DOM &nbsp;·&nbsp; Rust · Node.js &nbsp;·&nbsp; JSON-LD · microdata · RDFa · microformats &nbsp;·&nbsp; Playwright-compatible aria snapshots &nbsp;·&nbsp; robots.txt · sitemaps</sub>

</div>

---

## Overview

pestle reads a page once, in chunks as it arrives, and pulls out what a
*spec* names: CSS selectors pick the elements, and each value is read as
text, an attribute or markup (then, if asked, as a URL, a number or JSON).
In the same pass it can read the page's metadata, its JSON-LD, microdata,
RDFa and microformats (as JSON-LD of one shape), and its accessibility
tree as Playwright writes it. No DOM is built: memory is the stack of open
elements plus what you asked for. Malformed pages give what can be read;
nothing in a page can make it throw or crash.

Reach for pestle when you're building:

- **Scrapers and crawlers**: listings by CSS selector, links, images, feeds, embedded `__NEXT_DATA__`, bot-wall detection, robots.txt and sitemaps, 6–28× faster than the JavaScript parsers
- **Structured-data pipelines**: every schema.org item of a page, whatever the format, repaired the way sites mean it, or strictly per the W3C specifications
- **AI agents that read the web**: an accessibility snapshot with refs to act on, without a browser, with hidden prompt-injection text left out; pages stripped to a model's budget
- **Large or slow pages**: items handed over as each one closes, while the rest is still downloading

The tree is htmlparser2's and the matches are css-select's, so selectors
mean what they mean in cheerio. It was built for
Festimap's site crawler and is held byte for byte
to that pipeline's readers ([design](docs/design.md)).

## Installation

### Node.js / TypeScript (npm)

```bash
npm install @pestle-rs/node
```

&nbsp;→ [npmjs.com/package/@pestle-rs/node](https://www.npmjs.com/package/@pestle-rs/node). Prebuilt for Linux (glibc and musl), macOS and Windows, x64 and arm64; Node.js 18+; ESM, CommonJS and types included.

### Rust

The crates aren't on crates.io yet. Depend on a release tag:

```toml
# Cargo.toml
[dependencies]
pestle-html = { git = "https://github.com/the-haus/pestle-rs", tag = "v0.1.0" }
```

## Quick start

### Node.js

```ts
import { Extractor, scrape, ariaSnapshot, parseRobots } from "@pestle-rs/node";

// 1. What a spec names, streamed from the network.
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
});
const { data } = await agenda.stream(await fetch("https://venue.example/agenda"));
// data.events[0] → { title: "Nora & Co", url: "https://venue.example/e/nora", when: "za 14 nov 2026 · 20:00", price: 25, acts: ["Nora", "Kofi", "Ada"] }

// 2. Everything any page says of itself, with no spec.
const { page, structured } = await scrape(await fetch(url));
page.title; page.canonical; page.links; page.blocked;  // and every JSON-LD, microdata, RDFa, microformats item:
structured.items; structured.events;

// 3. A page for an agent: Playwright's ai-mode snapshot, without a browser.
const { snapshot, refs } = ariaSnapshot(html, { mode: "ai", concealed: true });

// 4. The crawler's half.
const robots = parseRobots(await (await fetch(new URL("/robots.txt", url))).text(), "mybot");
robots.allows(url);
```

The [Node.js guide](crates/bindings/pestle-node/README.md) covers every
function, option and recipe, and
[`examples/`](crates/bindings/pestle-node/examples) holds runnable scripts.

### Rust

```rust
use pestle_html::{Extractor, ExtractorOptions};
use serde_json::json;

let spec = json!({
    "title": "h1",
    "acts": ["ul.lineup > li"],
    "tickets": { "select": "a.tickets", "attr": "href", "url": "http" },
});
let x = Extractor::new(&spec, &Default::default(), ExtractorOptions::default())?;
let out = x.extract(html_bytes, Some("https://venue.example/e/1"));
// out.data == { "title": "Night", "acts": ["A", "B"], "tickets": "https://venue.example/t/1" }
```

`Extractor::session` streams chunk by chunk; `ExtractorOptions` switches on
the page pass, structured data, the aria snapshot, skeleton and strip. See
[`crates/pestle-html`](crates/pestle-html/README.md).

## What it reads

| | | |
| --- | --- | --- |
| **Spec fields** | CSS selectors → text, attributes, HTML, URLs, numbers, JSON, regexes; nested objects; items emitted as they close | [docs/spec.md](docs/spec.md) |
| **Page** | title, description, language, canonical, robots, refresh, every meta, alternates, feeds, calendars, icons, pagination, links with text, lazy images, headings, frames, e-mail, phone, embedded script JSON, bot walls | [docs/page.md](docs/page.md) |
| **Structured data** | JSON-LD, microdata, RDFa (OpenGraph, Dublin Core), microformats2 and classic, as one list of JSON-LD items plus a flat events view; W3C-conformant in strict mode | [docs/structured-data.md](docs/structured-data.md) |
| **Accessibility** | Playwright's `ariaSnapshot()` in default and `ai` modes, refs with CSS selectors, hidden text filtered | [docs/aria.md](docs/aria.md) |
| **Crawling** | robots.txt (RFC 9309, Google's leniency), sitemaps (indexes, gzip, text, RSS/Atom), streamed | [docs/crawling.md](docs/crawling.md) |
| **Charsets** | every WHATWG encoding, found as browsers find it, streaming | [docs/page.md#charsets](docs/page.md#charsets) |
| **For models** | festimap's `cleanForLearn` (a page within a token budget), template fingerprints | [docs/page.md#strip-and-skeleton](docs/page.md#strip-and-skeleton) |
| **Grammars** (opt-in) | any pest grammar compiled at run time, 3–5× faster than `pest_derive` | [docs/grammars.md](docs/grammars.md) |

## Performance

On 9 real venue and festival pages (4.2 MB), the same extraction:

| | MB/s |
| --- | --- |
| **pestle** (Node) | **703**, 1,056 on the thread pool |
| htmlparser2 + css-select | 107 |
| cheerio | 25 |
| **pestle** (Rust) | **~1,265** |
| lol_html | ~885 |
| scraper (html5ever) | ~181 |

Festimap's whole per-page read is 7.5× faster on one thread and 22.6× in
a batch on the pool. Accessibility snapshots are 44× faster than
Playwright's ([all numbers](docs/performance.md)).

## Documentation

| Area | Link |
|------|------|
| Node.js guide and API | [crates/bindings/pestle-node/README.md](crates/bindings/pestle-node/README.md) |
| The extraction spec and selectors | [docs/spec.md](docs/spec.md) |
| Page pass, charsets, strip and skeleton | [docs/page.md](docs/page.md) |
| Structured data and spec compliance | [docs/structured-data.md](docs/structured-data.md) |
| Accessibility snapshots for agents | [docs/aria.md](docs/aria.md) |
| robots.txt and sitemaps | [docs/crawling.md](docs/crawling.md) |
| pest grammars (opt-in) | [docs/grammars.md](docs/grammars.md) |
| Performance | [docs/performance.md](docs/performance.md) |
| Design, parity and testing | [docs/design.md](docs/design.md) |
| Rust: `pestle-html`, `pestle-grammar` | [crates/pestle-html](crates/pestle-html/README.md), [crates/pestle-grammar](crates/pestle-grammar/README.md) |
| Changelog | [CHANGELOG.md](CHANGELOG.md) |
| Release process | [RELEASING.md](RELEASING.md) |

## Development setup

**Prerequisites**

- Rust stable through `rustup` (pinned in [`rust-toolchain.toml`](rust-toolchain.toml); `rustfmt` and `clippy` come with it); 1.87 or newer
- A C compiler (mimalloc is compiled from vendored sources)
- Node.js 20+ with Corepack (the pinned Yarn 4 comes through it)

**Clone and bootstrap**

```bash
git clone https://github.com/the-haus/pestle-rs.git
cd pestle-rs
cargo test --workspace --exclude pestle-node
corepack enable
corepack yarn install --immutable
corepack yarn workspace @pestle-rs/node build
corepack yarn workspace @pestle-rs/node test
```

**Repository layout**

```
pestle-rs/
├── crates/
│   ├── pestle-html/          Tokenizer, tree rules, selectors, extraction, page pass,
│   │                         structured data, aria snapshots, robots.txt, sitemaps
│   ├── pestle-grammar/       pest grammars compiled at run time (opt-in)
│   └── bindings/
│       └── pestle-node/      @pestle-rs/node: N-API addon (src/), TypeScript API (ts/),
│                             tests, examples, benchmarks
├── bench/rust/               pestle against scraper, lol_html, tl, pest_derive
├── fixtures/                 festimap's pages (sites/), Playwright's snapshots (aria/)
├── docs/                     Guides and reference
└── scripts/                  Version checks and bumps, generators
```

## Building and testing

```bash
cargo test --workspace --exclude pestle-node    # Rust: tokenizer, tree, selectors, structured data, aria, pest parity
cargo test -p pestle-html --features grammar    # the same with grammar fields
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

cd crates/bindings/pestle-node
corepack yarn build              # or build:grammar, with pest grammars
corepack yarn typecheck
corepack yarn test               # API, structured data, junk input, differential tests, examples, packaging
corepack yarn smoke:install      # pack, install into a fresh project, use from ESM, CJS and TypeScript
corepack yarn shootout           # against the JavaScript parsers (pages in bench/.work/)
```

The conformance suites run when checked out into `bench/.work/suites`
([how](docs/structured-data.md#spec-compliance)).

## CI/CD

Every push and pull request runs the full quality gate on Linux, macOS and
Windows; a `v*` tag publishes to npm and GitHub Releases.

| Workflow | Purpose |
|----------|---------|
| [`ci`](.github/workflows/ci.yml) | `cargo fmt`, `clippy` (with and without grammars), `cargo test` on Linux, macOS and Windows; version agreement, crate packaging, `actionlint` |
| [`node`](.github/workflows/node.yml) | Build, typecheck and test `@pestle-rs/node` on three OSes and Node 20, 22 and 24; `npm pack`, then install the tarball into a fresh project; the grammar build |
| [`suites`](.github/workflows/suites.yml) | W3C JSON-LD 1.1 and RDFa 1.1 suites and the microformats suite at pinned commits; weekly too |
| [`release`](.github/workflows/release.yml) | Tag-driven: versions agree with the tag, the full gate in release mode, CHANGELOG notes, then npm and a GitHub release with checksums |
| [`release-node`](.github/workflows/release-node.yml) | Build the eight platform binaries natively (glibc, musl, macOS, Windows; x64 and arm64), test each where it was built, pack, install on every platform with Node 18, then publish with provenance |

See [RELEASING.md](RELEASING.md) for cutting a release.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first.

- Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and the tests above before pushing
- Keep festimap parity: text, skeleton and strip outputs must stay byte for byte; add new outputs beside them
- A bug fix comes with a test that fails without it

## License

MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)), like pest.
`crates/pestle-grammar/src/vm.rs` derives from `pest_vm`.

---

<div align="center">
  <sub>
    <a href="https://www.npmjs.com/package/@pestle-rs/node">npm</a> ·
    <a href="crates/bindings/pestle-node/README.md">Node.js guide</a> ·
    <a href="docs/">Docs</a> ·
    <a href="https://github.com/the-haus/pestle-rs/releases">Releases</a> ·
    <a href="https://github.com/the-haus/pestle-rs/issues">Issues</a>
  </sub>
  <br>
  <sub>Built in Rust for Festimap. One pass, no DOM.</sub>
</div>
