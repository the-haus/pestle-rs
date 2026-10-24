# The extraction spec

A spec says what to read from a page: an object of **fields**, each a CSS
selector plus how to read the value. It is plain data (JSON), so it can be
stored, generated (by a model, for instance) and validated before use.

```ts
import { Extractor } from "@pestle-rs/node";

const agenda = new Extractor({
  title: "h1",                                         // the text of the first <h1>
  image: "meta[property='og:image']@content",          // an attribute
  events: {
    select: "article.event",
    all: true,                                         // every match, as an array
    fields: {                                          // an object per match, relative to it
      title: "h2",
      url: { select: "h2 a", attr: "href", url: "http" },
      when: ".date",
      acts: ["ul.acts > li"],                          // every match, as an array of strings
      price: { select: ".price", number: true },       // "€ 25,-" → 25
      soldOut: { select: ".sold-out", exists: true },  // true / false
    },
  },
});
```

The same spec in Rust is a `serde_json::Value` handed to
`pestle_html::Extractor::new`.

## Field keys

A field is a selector string, `[selector]` for every match, or an object:

| key | what |
| --- | --- |
| `select` | CSS selector, relative to the field's scope; absent: the scope element itself |
| `all` | every match (an array), not only the first with a value |
| `limit` | with `all`: at most this many |
| `attr` | read this attribute instead of the text |
| `text` | `"visible"` (default: as a reader sees it), `"raw"` (all text, as is, scripts' too), `"own"` (only the element's own text) |
| `html` | read the element's outer HTML |
| `exists` | `true` when something matches, else `false` |
| `url` | resolve against the page (`true`), or only as an http(s) page without `#…` (`"http"`; other schemes give no value) |
| `pattern` | a regex (case-insensitive unless it starts with `(?`): its first group, else its match; no match is no value |
| `max` | cut the value to this many characters |
| `collapse` | collapse whitespace (default: for visible and own text and attributes) |
| `number` | read as a number (`€ 25,-` is 25, `1.234,50` is 1234.5) |
| `json` | parse as JSON (JSON-LD's CDATA, comments and stray control characters tolerated) |
| `parse` | parse with a pest grammar: `"name"`, `"name.rule"`, or `{ grammar, rule, find, all, limit, lists, numbers, byRule, trim, tree }` ([grammars](grammars.md); needs the `grammar` build) |
| `fields` | an object per match, its fields relative to the match |
| `default` | the value when nothing matched (otherwise `null`, or `[]` with `all`) |
| `emit` | top-level, with `all`: hand items over as they complete instead of keeping them ([streaming](#streaming-and-emitted-items)) |

**Shorthands.** `"h1"` is `{ "select": "h1" }`;
`"meta[property=og:image]@content"` reads an attribute; `["li.act"]` is
every match; `["li.act@data-id"]` every match's attribute.

## How values are read

- **Text** is what a reader sees: scripts, styles and templates are left
  out, block elements and `<br>` separate words, whitespace is collapsed.
  It is festimap's `textOf`, byte for byte.
- **Attributes** are decoded (`&amp;` is `&`) and collapsed.
- **URLs** resolve against the page's URL and its `<base href>`.
- A field with `all` keeps the matches that have a value; one without
  `all` takes the first match that has one.
- A value that cannot be read (bad JSON, a grammar that did not match) is
  `null`, and `errors` says why, with the field's path
  (`events[].when: …`). An extraction never throws because of the page.

## Selectors

Supported: type, `*`, `#id`, `.class`, every attribute operator (with `i`/`s`
and HTML's case-insensitive attributes), all four combinators, `:not()`,
`:is()`/`:where()`, `:first-child`, `:first-of-type`, `:nth-child()`,
`:nth-of-type()`, `:root` and `:scope`. Inside `fields`, a selector is
relative to its scope, as css-select makes it (`> li`, `a b` = `:scope a b`).

Some conditions can only be decided by what comes later in the document:
`:has()`, `:contains()`/`:icontains()`, `:empty`, `:last-child`,
`:only-child`, `:last-of-type`, `:only-of-type`, `:nth-last-*`. These work on
the selected element itself; pestle keeps the match's place in document
order and settles it when the element (or its parent) closes. On an
ancestor (`article:has(.x) h2`) they are a compile error. Write
`{ select: "article:has(.x)", fields: { t: "h2" } }` instead, or compile with
`lenient: true` to leave that one field out (listed in
`extractor.warnings`).

Pseudo-elements and live-page states (`:hover`, `:checked`) are errors too.

The tree selectors run on is htmlparser2's (`<li>` closes an open `<li>`,
`<div>` closes an open `<p>`, stray end tags are ignored), and the elements
a selector matches are css-select's. A differential test holds pestle to
htmlparser2 12 + css-select 7 on real pages and on thousands of randomly
broken documents.

## Errors

A spec that does not compile throws `SpecError` (Node) or returns
`Err(SpecError)` (Rust), with every problem and its field:

```ts
try {
  new Extractor({ title: "h1[" });
} catch (e) {
  if (e instanceof SpecError) console.error(e.errors); // ["title: selector: …"]
}
```

`lenient: true` leaves out the fields that do not compile instead, and
lists them in `extractor.warnings`: useful when specs come from a model.

## Streaming and emitted items

`extract(html)` reads a whole document; `stream(source)` and `session()` read
it chunk by chunk as it arrives, in the same memory: the stack of open
elements plus what the spec keeps.

A top-level field with `all: true, emit: true` keeps nothing: each item is
handed over as soon as its element closes, through `onItem` or
`session.take()`. A 100,000-event listing reads in the memory of one event:

```ts
const x = new Extractor({
  events: { select: "article.event", all: true, emit: true, fields: { title: "h2", when: ".date" } },
});
await x.stream(await fetch(url), { onItem: ({ field, value }) => save(value) });

// or by hand:
const session = x.session({ url });
for await (const chunk of body) {
  session.write(chunk);
  for (const item of session.take()) save(item.value);
}
const { stats } = session.end();
for (const item of session.take()) save(item.value); // the items the end completed
```

## Limits

| option | default | what happens past it |
| --- | --- | --- |
| `maxDepth` | 1024 | deeper start tags are ignored (`stats.tooDeep` counts them) |
| `maxCapture` | 4 MiB | one value's text is cut there |

Every other reader (page, structured data, snapshot) has bounds of its own
and says in its `warnings` what it cut.
