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
| `select` | CSS selector: the elements under the field's scope, matched in the page's context ([how](#a-fields-selector-under-its-scope)); absent: the scope element itself |
| `all` | every match (an array), not only the first with a value |
| `limit` | with `all`: at most this many |
| `attr` | read this attribute instead of the text |
| `text` | `"visible"` (default: as a reader sees it), `"lines"` (as visible, a line per block), `"raw"` (all text, as is, scripts' too), `"own"` (only the element's own text) |
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
| `closest` | under a scope: when nothing under it has a value, the nearest ancestor's (`true`, or `{ depth }`: 8 levels by default; [how](#values-stated-once-above-the-rows-closest)) |
| `default` | the value when nothing matched (otherwise `null`, or `[]` with `all`) |
| `emit` | top-level, with `all`: hand items over as they complete instead of keeping them ([streaming](#streaming-and-emitted-items)) |

**Shorthands.** `"h1"` is `{ "select": "h1" }`;
`"meta[property=og:image]@content"` reads an attribute (in an object too:
`{ select: "section@data-day", closest: true }`); `["li.act"]` is every
match; `["li.act@data-id"]` every match's attribute.

## How values are read

- **Text** is what a reader sees: scripts, styles and templates are left
  out, block elements and `<br>` separate words, whitespace is collapsed.
  It is festimap's `textOf`, byte for byte.
- **Text as lines** (`text: "lines"`) is the same text with a newline
  where a block (a heading, a paragraph, a list item, a table row) or a
  `<br>` breaks it: spaces collapsed within each line, empty lines left
  out, a table's cells on their row's line, inline elements inside a
  word (`<b>Nora</b>s` is `Noras`). `<h1>Title</h1><p>One</p><ul><li>a<li>b</ul>`
  reads `"Title\nOne\na\nb"`. The page pass gives the whole page's
  text so (`page: { text: true }`, [page](page.md)), for a model's
  prompt, or to find a quote that never runs across two blocks.
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
`:nth-of-type()`, `:root` and `:scope`.

### A field's selector under its scope

Inside `fields`, a selector picks elements *under* the scope, each matched
in the page's context, as the DOM's `element.querySelectorAll` matches (and
css-select with `relativeSelector: false`): under a `.card`, `.card h3` is
the card's `h3`, and under an `li` inside a `section`, `section b` is the
`li`'s `b`. The scope itself is never selected. To keep to the scope, say
so: `:scope h3` is an `h3` under it whatever is above, `> li` (`:scope >
li`) its children. Inside `:has()` a selector stays relative to the element
tested: `article:has(ul li)` wants the `ul` inside the `article`.

### Values stated once above the rows (`closest`)

A timetable often states a day or a stage once, above its rows. A field
with `closest` reads, when nothing under its scope has a value, the scope's
ancestors, nearest first, up to `depth` levels (8): in each, the first
match under it, as its `querySelector` would find it, else the ancestor
itself when it matches. A match without a value (an attribute it lacks) is
passed over.

```ts
const sets = new Extractor({
  sets: { select: "li.set", all: true, fields: {
    act: "b",
    day: { select: "section@data-day", closest: true },       // <section data-day="fri"> around the list
    stage: { select: "h3", closest: { depth: 3 } },            // an <h3> beside the list
  } },
});
```

A nearer ancestor wins over a farther one, and an ancestor's first match
may be in another row (a list's first `<h3>`, wherever it is in the list).
What an ancestor holds is sure only once it has closed, so a row waits for
it: an `emit` item is handed over when the ancestor that settles it has
closed, and its place in document order is kept. `closest` is for a field
under a scope, with a selector, reading one value (not `all`, not
`fields`); it cannot wait on later siblings (`:last-child` and its kin).

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
handed over as soon as its element closes (or, with a `closest` field, the
ancestor that settles it), through `onItem` or `session.take()`. A 100,000-event listing reads in the memory of one event:

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
