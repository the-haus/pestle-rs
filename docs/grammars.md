# pest grammars (optional)

pest grammars are behind a feature, so the default build (and binary) has no
grammar engine: 3.6 MB instead of 4.7 MB for the Node addon. Build with
`corepack yarn build:grammar` in `crates/bindings/pestle-node` (cargo:
`--features grammar` on `pestle-node` or `pestle-html`) to get `Grammar`, a
field's `parse` and `setCallLimit`. In a build without them `hasGrammars()`
is `false` and they throw an error that says so. The published npm package
is the default build.

Without grammars, a field keeps its value as written, or reads it with
`number`, `json` or `pattern` ([spec](spec.md)); normalising values (dates,
prices, lineups) is left to what consumes the output.

```ts
import { Extractor, Grammar } from "@pestle-rs/node";

const date = new Grammar(`
  WHITESPACE = _{ " " }
  when    = { weekday? ~ #day = day ~ #month = month ~ (#year = year)? ~ ("·" ~ #time = time)? }
  weekday = @{ ("ma" | "di" | "wo" | "do" | "vr" | "za" | "zo") ~ "."? }
  day     = @{ ASCII_DIGIT{1,2} }
  month   = @{ LETTER+ }
  year    = @{ ASCII_DIGIT{4} }
  time    = @{ ASCII_DIGIT{2} ~ ":" ~ ASCII_DIGIT{2} }
`);
const x = new Extractor(
  { when: { select: ".date", parse: { grammar: "date", find: true, numbers: ["day", "year"] } } },
  { grammars: { date } },
);
// → { when: { day: 14, month: "nov", year: 2026, time: "20:00" } }
```

`new Grammar(src)` gives `.parse()` (pest's pairs: `rule`, `tag`, `text`,
`children`, `find(tag)`; with `{ recover }` it goes on through errors),
`.shape()` and `.shapeAll()` (many values in one call), `.find()`,
`.findAll()` and `.matches()`. Offsets count UTF-16 units, as JavaScript
strings do.

## Semantics

A grammar means exactly what it means to `pest_derive`. `pest_meta` parses,
validates and optimises it; pestle compiles the result for its own engine,
which keeps pest's semantics (pairs, tags, atomicity, implicit whitespace,
the `PUSH`/`POP` stack) and drops what a successful parse never needs.

- **What it skips.** It doesn't track every attempt for error messages, box
  state through every combinator, or ask the OS for stack room on every call.
- **What it does instead.** Single-character alternatives become byte classes,
  and a choice skips alternatives whose first byte can't match. Literal and
  class sequences, `(!class ~ ANY)*` and implicit whitespace run as tight
  scans. Silent rules are inlined.
- **Errors.** A parse that fails runs again on pest's own `ParserState`, so
  error messages are pest's, word for word.

Tests hold it to the official `pest_vm`: identical pairs, tags and errors on
pest's JSON, TOML, HTTP and SQL grammars, plus a stack/tag/atomicity grammar,
over more than 4,000 randomly mutated inputs.

pest's own quirk applies: a tag tags the last pair made, so
`#year = year?` with no year tags the pair before it. Write
`(#year = year)?`.

## Error recovery

pest has none. Name the rules that may recover and the strings to pick up
again at:

```ts
const tree = lineup.parse(text, { recover: { rules: ["act"], sync: [";"], consume: true } });
tree.recovered; // [{ rule: "act", start, at, end, error: { message: "1:23: act broke here, expected time", … } }]
```

A named rule that breaks after matching something skips to the next
synchronising string. An `ERROR` pair stands for what was skipped, and the
parse goes on, so one bad act in a lineup costs that act, not the lineup. A
rule that fails at its first character is no error: that's how a
repetition ends.

`setCallLimit(n)` caps backtracking. Deep recursion always fails with an
error rather than crashing.

## Speed

| | pestle | the others |
| --- | --- | --- |
| 0.5 MB of JSON, full tree into JS | 7.2 ms | peggy 20.6 ms (2.9×), Chevrotain 94 ms (13×) |
| 0.5 MB of JSON, natively | 6.8 ms | `pest_derive` 22 ms (3.3×), `pest_vm` 86 ms (12.6×) |
| a short date, natively | 141 ns | `pest_derive` 691 ns (4.9×) |
| 10,000 short dates into JS objects | 548 ns each (`shapeAll`) | peggy 782 ns (1.4×) |
| one call each | 715 ns | peggy 782 ns (1.1×) |

`JSON.parse` (1.4 ms for that JSON) is faster than any grammar engine,
because it's a hand-written parser for one language. One short call at a
time, crossing into Rust costs about as much as the parse, so batch with
`shapeAll` (or let an extraction run the grammar in Rust, per value).
