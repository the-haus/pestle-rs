# Structured data

With `structured: true` (or `extractStructured`, or `scrape`), the same pass
reads JSON-LD, microdata and RDFa (OpenGraph and Dublin Core included), plus
microformats, and gives **every item of every type** (products, offers,
organizations, people, places, articles, recipes, FAQs, breadcrumbs,
reviews, events …, and whatever vocabulary a page uses) as **JSON-LD of
one shape**, whichever format the page used:

```ts
import { extractStructured } from "@pestle-rs/node";

const { items, events, warnings } = extractStructured(html, { url });
const venues = extractStructured(html, { url, types: ["Place"] }).items; // schema.org subtypes count
```

```json
{ "@context": "https://schema.org", "@type": "MusicEvent", "@id": "https://venue.example/e/nora",
  "@format": "microdata", "name": "Nora & Co", "startDate": "2026-11-14T20:00",
  "location": { "@type": "Place", "name": "Paradiso" }, "offers": { "@type": "Offer", "price": "25" } }
```

The same event in JSON-LD, microdata or RDFa gives the same item (a test
holds the three to it). Names are schema.org's short names, nested items
are embedded (each once per item; later references are its `@id`),
enumerations are member names (`EventScheduled`), relative URLs are
absolute, and `@context` lists any other vocabulary an item uses
(`["https://schema.org", { "og": "http://ogp.me/ns#" }]`). `@format`
says where it came from (JSON-LD processors ignore it). Values are as the
page states them: dates, prices and lineups are not rewritten. Microformats
items keep their own vocabulary (`h-event`) and get no `@context`.

## Events, flattened

Besides `items`, `events` gives every event flat, with the fields a calendar
needs (name, dates, status, location with address and coordinates,
performers, organizer, offers, keywords), each as the page states it. The
same event stated twice (in JSON-LD and microdata, or twice in one block)
is one event, its formats listed in `formats`.

## Options

| option | default | what |
| --- | --- | --- |
| `jsonLd`, `microdata`, `rdfa`, `microformats` | all `true` | which formats to read |
| `types` | all | keep only items of these types, wherever they are (`"Event"` keeps `Festival`) |
| `flatten` | `false` | also list every item that sits inside another (a product's offer, an event's place) as an item of its own |
| `events` | `true` | the flat events view |
| `mergeEvents` | `true` | one event for the same event stated twice |
| `raw` | `false` | also each format in its own standard form (the JSON-LD blocks, one entry per `<script>` in page order, `null` for one that cannot be read; microdata's WHATWG JSON, RDFa's triples, mf2 JSON) |
| `arrays` | `false` | every property value as an array |
| `detailed` | `false` | literals with language/datatype as `{ "@value", … }`, links as `{ "@id" }` |
| `embed` | `true` | embed the item a link (`@id`, `itemid`, an RDFa subject) names |
| `context` | `true` | give every item its `@context` |
| `defaultVocab` | schema.org | the vocabulary of JSON-LD that states no `@context` (`""`: none) |
| `lenient` | `true` | read what sites get wrong as they mean it (below); `false`: exactly as each specification has it |

## What sites get wrong, and what pestle does with it

| on the page | read as |
| --- | --- |
| broken JSON: trailing or missing commas, comments, single quotes, “smart quotes”, unquoted keys and values (`url: https://…`, `width:3px`), `True`/`None`/`NaN`, raw newlines, unescaped `"` inside text, entity-escaped or CDATA- or comment-wrapped blocks, truncated blocks, several objects glued together | the JSON meant (each repair listed in `warnings` with its block's index in `raw.jsonLd`, `json-ld[1]: trailing commas`; `raw.jsonLdRepaired` lists the blocks repaired) |
| `text/ld+json`, `application/json+ld`, a type with a charset | JSON-LD |
| `@context` missing, `http`/`https`, trailing `/`, `schema.org`, the docs URL, `{ "@vocab": … }`; `type`/`id` without `@`; `schema:` or full-IRI names | schema.org |
| names in the wrong case or style: `musicevent`, `Start_Date`, `door time`, `Music Event` | `MusicEvent`, `startDate`, `doorTime` |
| `"@type": "Event, MusicEvent"` | two types |
| HTML and character references in JSON-LD text (`<b>Nora</b> &amp; Co`) | plain text (`Nora & Co`) |
| empty values (`"url": ""`, `content=""`), `null`, `{}`, items with nothing in them | left out |
| `itemtype` without a scheme, with a trailing `/`, or without `itemscope`; `itemprop` as a full IRI; data-vocabulary.org | schema.org |
| `<a itemprop=url>` without `href`, `<img itemprop=image>` without `src` | no value (not the page's own address) |
| a URL property in `content` (`<meta itemprop=image content="/a.jpg">`, `og:image`) | the absolute URL |
| RDFa `typeof`/`property` without `vocab`; `vocab` in the wrong case; undeclared `schema:`/`og:`; an unknown prefix | schema.org (an unknown prefix is dropped) |
| HTML link types (`rel=stylesheet`, `icon`, `preload`) | no data |
| unknown types and properties | kept, as written: nothing is lost |

Every reader is bounded (text per value, triples, nodes, nesting, blocks of
JSON-LD and their size, nodes written per item) and says in `warnings`
what it cut. A reference graph that would multiply when written out stays
small; a page of 20,000 items reads in about 0.2 s.
`crates/pestle-html/tests/edge_cases.rs` is a corpus of faulty pages with
the JSON-LD each must give; it also covers deep nesting, huge values,
reference bombs and 3,000 random mutations of real markup.

## Spec compliance

`lenient: false` reads every format exactly as its specification has it: no
repairs, no default vocabulary, no guessing. Measured on the official test
suites (run in CI by the [`suites`](../.github/workflows/suites.yml)
workflow, at pinned commits):

| format | specification | suite | pestle | reference |
| --- | --- | --- | --- | --- |
| JSON-LD | JSON-LD 1.1 Processing Algorithms (expansion, node map, toRdf) | W3C json-ld-api toRdf, 443 tests in scope | **443** | jsonld.js 434 |
| RDFa | RDFa 1.1 Core + HTML+RDFa 1.1 (property copying, lists, `@inlist`) | W3C RDFa test suite, HTML5 | **170/170** | rdfa-streaming-parser 170 |
| microformats | microformats2 parsing + value-class pattern + classic formats with the include pattern | microformats/tests, 140 cases | **139** | microformats-parser 125 |
| microdata | WHATWG HTML microdata (`itemref`, URL properties, `itemid`) | a WHATWG-algorithm oracle on 2,000 generated documents | identical | |

The JSON-LD tests out of scope are those for JSON-LD 1.0 processing mode,
`rdfDirection`, generalized RDF and `expandContext`. In strict mode only
schema.org's context is known (built in); nothing is fetched, so a block with
another remote context is left out, as is any block that is not valid
JSON-LD. Either way `warnings` names the block and the spec's error code
(`json-ld[0]: a block was left out: invalid @id value`). Graphs are compared after RDFC-1.0
canonicalization.

The one microformats case pestle fails, `value-dt`, expects a timezone as
`+00:00`, which contradicts the value-class pattern ("If the offset uses
XX:YY format, remove the colon") and three other cases of the suite. pestle
keeps htmlparser2's tree for selectors; for microformats it follows HTML's
tree construction where the two differ in a way that matters: an `<a>`
inside an open `<a>` closes the outer one (the adoption agency algorithm).

To run the suites locally, check them out into `bench/.work/suites` (the
commits are in [`suites.yml`](../.github/workflows/suites.yml)), then:

```sh
cargo test -p pestle-html --release --test jsonld_suite --test mf_suite -- --nocapture
cd crates/bindings/pestle-node && node bench/rdfa-suite.mjs && node bench/jsonld-suite.mjs && node bench/mf-suite.mjs
```
