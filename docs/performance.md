# Performance

Measured on an M-series Mac (Node 24, Rust stable) on 9 real Dutch venue and
festival pages (4.2 MB), with the published build (on macOS: the system
allocator, see [below](#allocator)). The scripts are
`crates/bindings/pestle-node/bench/{bench,shootout}.mts` and
`bench/rust/src/bin/html.rs`; any pages saved in `bench/.work/` (not
committed) are used.

```sh
cd crates/bindings/pestle-node
corepack yarn build
corepack yarn shootout   # against the JavaScript parsers
corepack yarn bench      # against festimap's per-page read, robots.txt and sitemaps
cargo run --release -p pestle-bench --bin html   # against the Rust parsers
```

## Against JavaScript, the same extraction

Links with their text, headings, images, title:

| | MB/s | |
| --- | --- | --- |
| **pestle** | **703** | 1,056 MB/s with `extractAsync` on the thread pool |
| htmlparser2 SAX, hand-written, no selectors | 142 | pestle 5.0× faster |
| node-html-parser | 110 | 6.4× |
| htmlparser2 + css-select (festimap today) | 107 | 6.6× |
| linkedom | 44 | 16× |
| cheerio (parse5) | 25 | 28× |

## Festimap's actual per-page read

The htmlparser2 DOM, css-select queries, regex readers for JSON-LD, links
and feeds, and a second parse for the skeleton, with the same answers on
every field:

| | TypeScript today | pestle |
| --- | --- | --- |
| 9 pages, one thread | 108 ms | 14.5 ms: **7.5×** |
| 32 pages, async | 405 ms on the JS thread | 18 ms on the pool: **22.6×**, JS thread free |
| `cleanForLearn` (1.2 MB page) | 13.0 ms | 0.91 ms: **14.3×**, identical output |
| memory | the page as a DOM on the JS heap | no DOM |

## Against Rust parsers, natively

| | MB/s |
| --- | --- |
| **pestle** | **~1,265** |
| tl (zero-copy, not spec-compliant) | ~1,120 |
| lol_html (Cloudflare's streaming rewriter) | ~885 |
| scraper (html5ever) | ~181 |

## The rest

- **Structured data**, natively: 0.1–1.6 ms a page (200 MB/s to 2 GB/s),
  all four formats read in the same pass as the rest
  (`cargo run --release -p pestle-html --example structured_bench`).
- **Accessibility snapshots**: 44× faster than Playwright's from one
  thread, 145× on the pool ([aria](aria.md#speed)).
- **Crawling**: robots.txt checks 6×, sitemaps 1.7× faster than festimap's
  TypeScript ([crawling](crawling.md#parity)).
- **Grammars**: 3–5× faster than `pest_derive` ([grammars](grammars.md#speed)).

## How the HTML side gets there

- Elements point into the input instead of copying it. At a chunk's end, the
  few still open are copied; a whole document is never copied.
- Only attributes the spec can read are kept.
- Known tags get small numbers, so the tree rules compare integers.
- Each field is tested only on tags its selector can match.
- Text is decoded only when something reads it.
- Results are compact records, not hash maps.

## Allocator

The Node addon uses mimalloc on Linux and Windows. On macOS it uses the
system allocator: there mimalloc keeps each thread's heap in fixed TLS
slots of the process, so two addons that each bundle it (pestle and
`@wikidata-rs/node`, say) took each other's heaps and crashed. The system
allocator costs about 10% on macOS (785 → 703 MB/s above);
`test/coexist.test.ts` loads two copies of the addon in one process to keep
it that way.
