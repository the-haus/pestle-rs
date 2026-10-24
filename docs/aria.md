# Accessibility snapshots (pages for AI agents)

`ariaSnapshot(html, { url })` (or `aria: true` on any pass) gives the page's
accessibility tree as YAML: the snapshot Playwright's `page.ariaSnapshot()`
takes in a browser, with roles, accessible names, states, links' URLs and
form values. It needs no browser.

```ts
import { ariaSnapshot } from "@pestle-rs/node";

const html = `<nav aria-label="Main"><a href="/agenda">Agenda</a></nav>
  <main><h1>Nora &amp; Co</h1><input aria-label="Tickets" value="2"><button disabled>Buy</button></main>`;

ariaSnapshot(html).snapshot;
// - navigation "Main":
//   - link "Agenda":
//     - /url: /agenda
// - main:
//   - heading "Nora & Co" [level=1]
//   - textbox "Tickets": "2"
//   - button "Buy" [disabled]

const { snapshot, refs } = ariaSnapshot(html, { mode: "ai" });
// - generic [active] [ref=e1]:
//   - navigation "Main" [ref=e2]:
//     - link "Agenda" [ref=e3] [cursor=pointer]:
//       - /url: /agenda
//   - main [ref=e4]:
//     - heading "Nora & Co" [level=1] [ref=e5]
//     - textbox "Tickets" [ref=e6]: "2"
//     - button "Buy" [disabled] [ref=e7]
// refs.e7: "body > main:nth-child(2) > button:nth-child(3)"
```

`mode: "ai"` is Playwright's snapshot for agents: every element that can be
acted on gets a ref, generic containers are kept, and names that repeat
their content are cut. Each ref comes with a CSS selector, so a browser can
act on it. `depth` limits how deep it goes. `ariaSnapshotAsync` runs on the
thread pool.

## Hidden text and prompt injection

`concealed: true` also leaves out what the page hides from sight but keeps
in its markup:

- no size (`font-size: 0`), no opacity, far off screen, clipped away;
- hidden by a framework's class (`hidden`, `d-none`, `is-hidden`, unless a
  responsive class such as `md:block` shows it again).

Text meant for screen readers (`sr-only`) stays. This is for pages read by a
model: hidden text is where instructions get slipped in.
[`examples/agent-input.ts`](../crates/bindings/pestle-node/examples/agent-input.ts)
shows it on a page with two such instructions.

## Fidelity

Its rules are Playwright's, ported from its injected script: implicit and
explicit roles, presentational roles inherited and resolved, the accessible
name computation (`aria-labelledby`, `aria-label`, labels, `alt`, captions,
legends, an SVG's `<title>`, names from content), states (checked, disabled,
expanded, invalid, level, pressed, selected), and the passes that distill
the tree. [`fixtures/aria`](../fixtures/aria) holds 36 pages with the
snapshots Playwright 1.64.1 (Chromium) took of them, and tests hold pestle to
them in both modes. On those 36 real venue and festival pages (7.2 MB), each
with its own `<style>` blocks:

| | identical to Playwright | lines in agreement |
| --- | --- | --- |
| default mode | **36/36** | 100% |
| `ai` mode | 28/36 (29 with ref numbers aside) | 99.8% |

The `ai` differences are refs. Playwright gives one only to an element that
layout sizes, and pestle has no layout.

What only a browser knows is read from the markup:

- **Hidden.** An element is hidden by `hidden`, `aria-hidden`, the browser's
  own style sheet (`<head>`, closed `<details>` and `<dialog>`, a drop-down's
  options), inline `display`/`visibility`/`content-visibility`, and the
  page's own `<style>` blocks (`styleBlocks`, on by default). Those blocks
  supply their `display`, `visibility`, `cursor` and `pointer-events` rules,
  with specificity, `!important` and media queries for a 1280×720 screen.
  A block applies to what comes after it.
- **Not read.** Style sheets the page links to are never fetched, CSS
  generated content counts as none, and scripts are not run. For a page that
  scripts build, take the snapshot of the HTML it renders to.
- **Refs in `ai` mode.** Playwright gives a ref only to an element with a
  size. pestle estimates size from content (text, controls, media, images
  that load, SVG shapes); a layout only style sheets give can differ.

It is kept bounded on hostile pages: nesting of any depth, `aria-owns`
chains and cycles, thousands of references to one label, huge or broken
style sheets.

## Speed

Node, on the same 36 pages:

| | time | |
| --- | --- | --- |
| Playwright `ariaSnapshot()`, the page already loaded | 899 ms (+ 804 ms to load the pages) | |
| **pestle**, default mode, one thread | **20.5 ms** (352 MB/s) | 44× faster (83× with loading) |
| **pestle**, default mode, thread pool | **6.2 ms** | 145× faster (275× with loading) |
| Playwright, `ai` mode | 1,154 ms (+ 807 ms load) | |
| **pestle**, `ai` mode, one thread | **38.7 ms** | 30× faster (51× with loading) |

On a single 842 kB page pestle takes 4 ms against Playwright's 108 ms
(+102 ms to load it); on small pages the gap grows to several hundred times.
