# Design

## One pass, no DOM

pestle reads a page once, in chunks as it arrives. A tokenizer feeds
htmlparser2's tree rules, and each element is matched against the compiled
spec as it opens; values are read when it closes. Memory is the stack of
open elements plus what the spec keeps, never a DOM. The page pass,
structured data, skeleton, strip and accessibility snapshot all ride the same
token stream, so asking for more costs little more than asking for less.

Malformed pages give what can be read: nothing in a page can make it throw
or crash. Every reader is bounded, and every native entry point catches
panics.

## Festimap parity

pestle is made for Festimap's site crawler and is a drop-in for the HTML
half of its `packages/sources/src/sites`. The tree it builds is
htmlparser2's, the elements a selector matches are css-select's, and its
text, skeleton and stripped page are festimap's (`textOf`, `skeletonPaths`,
`cleanForLearn`). Tests hold it to all of these, so recipes learned and
validated against today's pipeline mean the same thing here. New outputs
(the page pass, structured data, accessibility snapshots) are added beside
those, never by changing them.

## Why not parse HTML with pest

The first question was whether pest is the right kind of parser for this.
For HTML structure, it is not:

- **HTML is not a grammar.** Browsers and htmlparser2 parse it with an
  error-tolerant state machine: `<li>` closes an open `<li>`, `<div>` closes
  an open `<p>`, stray end tags are ignored, `<script>` holds raw text. A PEG
  grammar over raw markup either fails on the first thing it did not foresee
  (pest has no error recovery) or turns into a pile of regex-like rules.
- **pest cannot stream.** It needs the whole input as one `&str` and can
  backtrack anywhere in it.
- **Festimap already addresses data with CSS selectors.** Its recipes are
  selectors that a model learns and that are validated on held-out pages.
  What that needs is a fast, faithful selector engine, not a grammar.

pest fits the **values** better: small text languages such as
`za 14 nov 2026 · 20:00` or `€ 25,- / € 30,- deur`. pestle leaves those as
written by default (normalising them is a job for the consumer, a model in
Festimap's case); a pest grammar is there as an opt-in for a value language
of your own ([grammars](grammars.md)).

## Testing

- **Differential**: pestle against htmlparser2 12 + css-select 7 and
  festimap's readers, on festimap's fixtures, any pages saved in
  `bench/.work/`, and about 1,400 seeded random documents full of broken
  markup (misnested and unclosed tags, stray end tags, foreign content, raw
  text, entities). Selection, text, attributes, skeleton and strip must all
  be identical.
- **Conformance**: the W3C JSON-LD and RDFa suites and the microformats
  suite, in CI at pinned commits ([structured data](structured-data.md#spec-compliance)).
- **Playwright**: 36 pages with Playwright's own snapshots in
  `fixtures/aria`.
- **pest**: identical pairs, tags and errors to `pest_vm` over thousands of
  mutated inputs.
- **Robustness**: a fuzz test feeds random junk through every structured
  format; hostile pages (deep nesting, reference bombs, `aria-owns` cycles)
  stay bounded.
- **Packaging**: the built ESM and CommonJS entry points, two copies of the
  addon in one process, every example, and the packed tarball installed into
  a fresh project on every platform before anything is published.
