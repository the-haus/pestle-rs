# pestle-html

Streaming HTML extraction for scrapers and AI agents: CSS selectors matched
over a token stream, no DOM. Page metadata, links, embedded JSON, structured
data (JSON-LD, microdata, RDFa, microformats), Playwright-compatible
accessibility snapshots, robots.txt and sitemaps, all in one pass and in
any charset.

This is the engine behind [`@pestle-rs/node`](../bindings/pestle-node/README.md);
the [repository README](../../README.md) and [docs](../../docs) describe
what it reads.

```rust
use pestle_html::{Extractor, ExtractorOptions};
use serde_json::json;

let spec = json!({
    "title": "h1",
    "acts": ["ul.lineup > li"],
    "tickets": { "select": "a.tickets", "attr": "href", "url": "http" },
});
let x = Extractor::new(&spec, &Default::default(), ExtractorOptions::default()).unwrap();
let html = br#"<h1>Night</h1><ul class=lineup><li>A<li>B</ul><a class=tickets href="/t/1">Buy</a>"#;
let out = x.extract(html, Some("https://venue.example/e/1"));
assert_eq!(out.data, json!({ "title": "Night", "acts": ["A", "B"], "tickets": "https://venue.example/t/1" }));
```

[`examples/quickstart.rs`](examples/quickstart.rs) goes further: a streamed
document with emitted items, the page, structured data and aria snapshot in
one pass, robots.txt and a sitemap (`cargo run -p pestle-html --example quickstart`).
Output values are pestle's compact `Value`; `to_json()` turns one into a
`serde_json::Value`.

## What's in it

| | |
| --- | --- |
| `Extractor`, `Session` | a compiled [spec](../../docs/spec.md) (cheap to clone and share across threads); `extract` reads a whole document, `session` streams chunk by chunk |
| `ExtractorOptions` | switch on `page`, `structured`, `aria`, `skeleton` and `strip` in the same pass; `max_depth`, `max_capture`, `lenient` |
| `page` | [what any page says of itself](../../docs/page.md) |
| `structured` | [JSON-LD, microdata, RDFa, microformats](../../docs/structured-data.md) as JSON-LD items and flat events |
| `aria` | [accessibility snapshots](../../docs/aria.md), as Playwright's `ariaSnapshot()` |
| `robots`, `sitemap`, `routes` | [robots.txt, sitemaps and routes](../../docs/crawling.md) |
| `charset` | the WHATWG encoding sniffing browsers do |
| `selector`, `tree`, `text` | the selector engine (css-select's semantics), htmlparser2's tree rules, festimap's text |

## Features

- `grammar`: fields that parse their value with a pest grammar (`parse`),
  through [`pestle-grammar`](../pestle-grammar/README.md). Off by default.

## Install

Not on crates.io yet:

```toml
[dependencies]
pestle-html = { git = "https://github.com/the-haus/pestle-rs", tag = "v0.3.0" }
```

## License

MIT OR Apache-2.0.
